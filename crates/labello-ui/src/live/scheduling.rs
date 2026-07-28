impl LabelloApp {
    pub(crate) fn cache_current_assignment_availability(&mut self) {
        let Some(dataset_id) = self.work.availability.dataset_id.clone() else {
            return;
        };
        let Some(kind) = self.work.availability.kind.clone() else {
            return;
        };
        let Some(checked_at) = self.work.availability.checked_at else {
            return;
        };
        if !self.work.availability.resolved || self.work.availability.error.is_some() {
            return;
        }
        self.cache_assignment_availability(
            dataset_id,
            kind,
            self.work.availability.tasks.clone(),
            checked_at,
        );
    }

    pub(crate) fn cache_assignment_availability(
        &mut self,
        dataset_id: labello_domain::DatasetId,
        kind: labello_domain::AssignmentKind,
        tasks: std::collections::BTreeMap<labello_domain::TaskId, bool>,
        checked_at: labello_domain::Timestamp,
    ) {
        self.work.availability.cache.retain(|cached| {
            cached.dataset_id != dataset_id || cached.kind != kind
        });
        self.work
            .availability
            .cache
            .push(crate::app::CachedAssignmentAvailability {
                dataset_id,
                kind,
                tasks,
                checked_at,
            });
    }

    pub(crate) fn reset_assignment_availability_for_workspace(&mut self) {
        self.cache_current_assignment_availability();
        let cache = std::mem::take(&mut self.work.availability.cache);
        self.work.availability = Default::default();
        self.work.availability.cache = cache;
    }

    pub(crate) fn invalidate_assignment_availability(
        &mut self,
        dataset_id: Option<&labello_domain::DatasetId>,
    ) {
        if let Some(dataset_id) = dataset_id {
            self.work
                .availability
                .cache
                .retain(|cached| &cached.dataset_id != dataset_id);
            if self.work.availability.dataset_id.as_ref() == Some(dataset_id) {
                self.work.availability.checked_at = None;
            }
        } else {
            self.work.availability.cache.clear();
            self.work.availability.checked_at = None;
        }
    }

    pub(crate) fn restore_session_assignment_availability(&mut self) -> bool {
        let Some(kind) = self.assignment_kind() else {
            return false;
        };
        let dataset_id = self.config.dataset_id.clone();
        let Some(index) = self.work.availability.cache.iter().position(|cached| {
            cached.dataset_id == dataset_id && cached.kind == kind
        }) else {
            return false;
        };
        let cached = self.work.availability.cache[index].clone();
        let tasks_match = cached.tasks.len() == self.work.tasks.len()
            && self
                .work
                .tasks
                .iter()
                .all(|task| cached.tasks.contains_key(&task.task_id));
        let fresh = labello_domain::now()
            .signed_duration_since(cached.checked_at)
            .to_std()
            .is_ok_and(|age| age < ASSIGNMENT_AVAILABILITY_CACHE_TTL);
        if !tasks_match || !fresh {
            self.work.availability.cache.remove(index);
            return false;
        }
        self.work.availability.dataset_id = Some(dataset_id);
        self.work.availability.kind = Some(kind);
        self.work.availability.tasks = cached.tasks;
        self.work.availability.resolved = true;
        self.work.availability.checked_at = Some(cached.checked_at);
        self.work.availability.loading = false;
        self.work.availability.load_after_resolution = false;
        self.work.availability.refresh_after_load = false;
        self.work.availability.error = None;
        self.work.availability.last_attempt = Some(Instant::now());
        true
    }

    pub(crate) fn refresh_stats_if_due(&mut self) {
        if self.runtime.api.is_none()
            || self.datasets.metadata.is_none()
            || self.loading.stats
            || matches!(self.view, AppView::Setup)
            || !matches!(self.view, AppView::Stats)
        {
            return;
        }
        let due = self
            .datasets
            .last_stats_attempt
            .is_none_or(|last| last.elapsed() >= Duration::from_secs(3));
        if due {
            self.request_stats();
        }
    }

    pub(crate) fn refresh_assignment_availability_if_due(&mut self) {
        if self.work.availability.loading {
            return;
        }
        let Some(kind) = self.assignment_kind() else {
            return;
        };
        let mut context_changed = self.work.availability.dataset_id.as_ref()
            != Some(&self.config.dataset_id)
            || self.work.availability.kind.as_ref() != Some(&kind);
        if context_changed && self.restore_session_assignment_availability() {
            context_changed = false;
        }
        let due = context_changed
            || if self.work.availability.checked_at.is_some() {
                self.assignment_availability_cache_age()
                    .is_none_or(|age| age >= ASSIGNMENT_AVAILABILITY_CACHE_TTL)
            } else {
                self.work
                    .availability
                    .last_attempt
                    .is_none_or(|last| last.elapsed() >= ASSIGNMENT_AVAILABILITY_CACHE_TTL)
            };
        if due {
            self.request_assignment_availability();
        }
    }

    pub(crate) fn request_assignment_availability(&mut self) {
        let Some(kind) = self.assignment_kind() else {
            return;
        };
        if self.runtime.api.is_none() {
            return;
        }
        if self.work.availability.loading {
            self.work.availability.refresh_after_load = true;
            self.work.availability.last_attempt = None;
            return;
        }
        let dataset_id = self.config.dataset_id.clone();
        if self.work.availability.dataset_id.as_ref() != Some(&dataset_id)
            || self.work.availability.kind.as_ref() != Some(&kind)
        {
            self.work.availability.tasks.clear();
            self.work.availability.resolved = false;
            self.work.availability.checked_at = None;
            self.work.availability.error = None;
        }
        self.work.availability.dataset_id = Some(dataset_id.clone());
        self.work.availability.kind = Some(kind.clone());
        self.work.availability.loading = true;
        self.work.availability.last_attempt = Some(Instant::now());
        let request = self.request_identity(Some(dataset_id.clone()));
        self.queue_command(UiCommand::AssignmentAvailability {
            request,
            dataset_id,
            kind,
        });
    }

    pub(crate) fn refresh_ingest_if_due(&mut self) {
        if !self.loading.ingesting || self.loading.ingest_polling {
            return;
        }
        let Some(job_id) = self.loading.ingest_job_id.clone() else {
            return;
        };
        let due = self
            .loading
            .last_ingest_poll
            .is_none_or(|last| last.elapsed() >= Duration::from_millis(500));
        if due {
            self.loading.ingest_polling = true;
            let request = self.request_identity(Some(self.config.dataset_id.clone()));
            self.queue_command(UiCommand::PollIngest {
                request,
                dataset_id: self.config.dataset_id.clone(),
                job_id,
            });
        }
    }

}
