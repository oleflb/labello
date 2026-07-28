impl crate::app::LabelloApp {
    pub(crate) fn start_next_persistence_command(&mut self) {
        if self.runtime.persistence.active {
            return;
        }
        let now = Instant::now();
        let Some(index) = self
            .runtime
            .persistence
            .commands
            .iter()
            .position(|command| command.ready_at <= now)
        else {
            if let Some(delay) = self
                .runtime
                .persistence
                .commands
                .iter()
                .filter_map(|command| command.ready_at.checked_duration_since(now))
                .min()
                && let Some(ctx) = self.runtime.repaint_ctx.as_ref()
            {
                ctx.request_repaint_after(delay);
            }
            return;
        };
        let command = self
            .runtime
            .persistence
            .commands
            .remove(index)
            .expect("ready persistence command exists");
        self.runtime.persistence.active = true;
        let store = self.runtime.persistence.store.clone();
        let request = crate::app::RequestIdentity {
            auth_epoch: self.auth_epoch,
            workspace_epoch: self.workspace_epoch,
            request_id: 0,
            dataset_id: Some(self.config.dataset_id.clone()),
        };
        self.spawn_message(request, async move {
            crate::app::UiMessage::PersistenceFinished(Box::new(
                execute_persistence_command(store, command).await,
            ))
        });
    }

    pub(crate) fn handle_persistence_completion(&mut self, completion: PersistenceCompletion) {
        self.runtime.persistence.active = false;
        let command = completion.command();
        if self.runtime.persistence.identity.as_ref() != Some(&command.identity)
            || command
                .key()
                .is_some_and(|key| !command.identity.owns_key(key))
        {
            return;
        }
        match completion {
            PersistenceCompletion::Loaded { command, result } => {
                let key = command.key().expect("load command has a key").to_string();
                match *result {
                    Ok(Some(DraftRecord::Work(draft))) => {
                        if let (Some(identity), Some(assignment)) = (
                            self.runtime.persistence.identity.as_ref(),
                            self.work.assignment.as_ref(),
                        ) && key == work_draft_key(identity, &self.config.dataset_id, assignment)
                        {
                            self.runtime.persistence.work_ready =
                                Some(assignment.assignment_id.clone());
                        }
                        let validation = match (
                            self.work.assignment.as_ref(),
                            self.work.current_state.as_ref(),
                        ) {
                            (Some(assignment), Some(state)) if draft.key == key => {
                                validate_work_draft(
                                    &draft,
                                    self.runtime
                                        .persistence
                                        .identity
                                        .as_ref()
                                        .expect("identity exists"),
                                    &self.config.dataset_id,
                                    assignment,
                                    state,
                                    labello_domain::now(),
                                )
                            }
                            _ => DraftValidation::Conflict(
                                "The assignment changed before its draft finished loading."
                                    .to_string(),
                            ),
                        };
                        if matches!(validation, DraftValidation::Expired(_)) {
                            self.queue_persistence(PersistenceCommand::Delete(draft.key.clone()));
                        }
                        self.runtime.persistence.recovery =
                            Some(DraftRecovery::Work(draft, validation));
                        if matches!(
                            self.runtime.persistence.recovery,
                            Some(DraftRecovery::Work(_, DraftValidation::Valid))
                        ) {
                            self.recover_browser_draft();
                        }
                    }
                    Ok(Some(DraftRecord::Admin(draft))) => {
                        let validation = match self.datasets.admin_baseline.as_ref() {
                            Some(baseline) => validate_admin_draft(
                                &draft,
                                self.runtime
                                    .persistence
                                    .identity
                                    .as_ref()
                                    .expect("identity exists"),
                                &self.config.dataset_id,
                                baseline,
                            ),
                            None => DraftValidation::Conflict(
                                "The admin dataset changed before its draft finished loading."
                                    .to_string(),
                            ),
                        };
                        self.runtime.persistence.recovery =
                            Some(DraftRecovery::Admin(draft, validation));
                    }
                    Ok(None) => {
                        if let (Some(identity), Some(assignment)) = (
                            self.runtime.persistence.identity.as_ref(),
                            self.work.assignment.as_ref(),
                        ) && key == work_draft_key(identity, &self.config.dataset_id, assignment)
                        {
                            self.runtime.persistence.work_ready =
                                Some(assignment.assignment_id.clone());
                        }
                    }
                    Err(error) => self.storage_failure(error),
                }
            }
            PersistenceCompletion::Saved { command, result } => match result {
                Ok(()) => match &command.command {
                    PersistenceCommand::Save(record) => match record.as_ref() {
                        DraftRecord::Work(draft) => {
                            self.runtime.persistence.last_work_draft = Some((**draft).clone());
                        }
                        DraftRecord::Admin(draft) => {
                            self.runtime.persistence.last_admin_config = Some(draft.config.clone());
                        }
                    },
                    _ => unreachable!("saved completion has a save command"),
                },
                Err(error) => {
                    let key = command.key().expect("save command has a key").to_string();
                    self.storage_failure(format!("{key}: {error}"));
                    self.retry_persistence(command);
                }
            },
            PersistenceCompletion::Deleted { command, result } => match result {
                Ok(()) => {
                    let key = command.key().expect("delete command has a key");
                    if self
                        .runtime
                        .persistence
                        .last_work_draft
                        .as_ref()
                        .is_some_and(|draft| draft.key == key)
                    {
                        self.runtime.persistence.last_work_draft = None;
                    }
                    if self.runtime.persistence.last_admin_config.is_some()
                        && self
                            .runtime
                            .persistence
                            .identity
                            .as_ref()
                            .is_some_and(|identity| {
                                key == admin_draft_key(identity, &self.config.dataset_id)
                            })
                    {
                        self.runtime.persistence.last_admin_config = None;
                        self.runtime.persistence.admin_delete_desired = false;
                    }
                }
                Err(error) => {
                    let key = command.key().expect("delete command has a key").to_string();
                    self.storage_failure(format!("{key}: {error}"));
                    self.retry_persistence(command);
                }
            },
            PersistenceCompletion::GarbageCollected { result, .. } => {
                if let Err(error) = result {
                    self.storage_failure(error);
                }
            }
        }
    }

    fn queue_persistence(&mut self, command: PersistenceCommand) {
        let Some(identity) = self.runtime.persistence.identity.clone() else {
            return;
        };
        self.enqueue_persistence(QueuedPersistenceCommand {
            identity,
            command,
            attempt: 0,
            ready_at: Instant::now(),
        });
    }

    fn retry_persistence(&mut self, mut command: QueuedPersistenceCommand) {
        let Some(key) = command.key().map(str::to_string) else {
            return;
        };
        if self.runtime.persistence.commands.iter().any(|queued| {
            queued.key() == Some(&key)
                && matches!(
                    queued.command,
                    PersistenceCommand::Save(_) | PersistenceCommand::Delete(_)
                )
        }) {
            return;
        }
        command.attempt = command.attempt.saturating_add(1);
        let delay = retry_delay(command.attempt.saturating_sub(1));
        command.ready_at = Instant::now() + delay;
        self.runtime.persistence.commands.push_back(command);
        if let Some(ctx) = self.runtime.repaint_ctx.as_ref() {
            ctx.request_repaint_after(delay);
        }
    }

    fn enqueue_persistence(&mut self, command: QueuedPersistenceCommand) {
        let key = match &command {
            QueuedPersistenceCommand {
                command: PersistenceCommand::Load(key),
                ..
            }
            | QueuedPersistenceCommand {
                command: PersistenceCommand::Delete(key),
                ..
            } => Some(key.as_str()),
            QueuedPersistenceCommand {
                command: PersistenceCommand::Save(record),
                ..
            } => Some(record.key()),
            QueuedPersistenceCommand {
                command: PersistenceCommand::GarbageCollect(_),
                ..
            } => None,
        };
        if let Some(key) = key {
            self.runtime
                .persistence
                .commands
                .retain(|queued| match queued {
                    QueuedPersistenceCommand {
                        command: PersistenceCommand::Load(queued_key),
                        ..
                    } => {
                        !matches!(command.command, PersistenceCommand::Load(_)) || queued_key != key
                    }
                    QueuedPersistenceCommand {
                        command: PersistenceCommand::Save(record),
                        ..
                    } => {
                        !matches!(
                            command.command,
                            PersistenceCommand::Save(_) | PersistenceCommand::Delete(_)
                        ) || record.key() != key
                    }
                    QueuedPersistenceCommand {
                        command: PersistenceCommand::Delete(queued_key),
                        ..
                    } => {
                        !matches!(
                            command.command,
                            PersistenceCommand::Save(_) | PersistenceCommand::Delete(_)
                        ) || queued_key != key
                    }
                    QueuedPersistenceCommand {
                        command: PersistenceCommand::GarbageCollect(_),
                        ..
                    } => true,
                });
        }
        self.runtime.persistence.commands.push_back(command);
    }

    fn storage_failure(&mut self, error: String) {
        tracing::warn!(
            event = "browser_storage.failed",
            "browser persistence operation failed"
        );
        self.runtime.storage_error = Some(format!("Browser storage failed: {error}"));
    }
}
