impl LabelloApp {
    fn dispatch_migration_command(
        &self,
        api: Rc<dyn labello_client::LabelloApi>,
        command: UiCommand,
    ) -> Option<UiCommand> {
        match command {
            UiCommand::Migration {
                request,
                dataset_id,
                image_id,
                action,
                idempotency_key,
            } => self.spawn_message(request.clone(), async move {
                let result = match action {
                    crate::app::MigrationAction::SaveSkeleton(body) => {
                        api.save_migration_skeleton(&dataset_id, &image_id, body, &idempotency_key)
                            .await
                    }
                    crate::app::MigrationAction::AddSkeleton(body) => {
                        api.add_migration_skeleton(&dataset_id, &image_id, body, &idempotency_key)
                            .await
                    }
                    crate::app::MigrationAction::Exclude(body) => {
                        api.exclude_migration_target(&dataset_id, &image_id, body, &idempotency_key)
                            .await
                    }
                    crate::app::MigrationAction::Reopen(body) => {
                        api.reopen_migration_target(&dataset_id, &image_id, body, &idempotency_key)
                            .await
                    }
                    crate::app::MigrationAction::Revisit(body) => {
                        api.revisit_migration_target(&dataset_id, &image_id, body, &idempotency_key)
                            .await
                    }
                    crate::app::MigrationAction::StartPass(body) => {
                        api.start_migration_pass(&dataset_id, &image_id, body, &idempotency_key)
                            .await
                    }
                    crate::app::MigrationAction::Keep(body) => {
                        api.keep_migration_target(&dataset_id, &image_id, body, &idempotency_key)
                            .await
                    }
                    crate::app::MigrationAction::Confirm(body) => {
                        api.confirm_migration(&dataset_id, &image_id, body, &idempotency_key)
                            .await
                    }
                    crate::app::MigrationAction::Review(body) => {
                        api.review_migration(&dataset_id, &image_id, body, &idempotency_key)
                            .await
                    }
                }
                .map_err(|error| error.to_string());
                UiMessage::MigrationFinished {
                    request,
                    result: Box::new(result),
                }
            }),
            command => return Some(command),
        }
        None
    }
}
