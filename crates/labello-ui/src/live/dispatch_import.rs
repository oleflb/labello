impl LabelloApp {
    fn dispatch_import_command(
        &self,
        api: Rc<dyn labello_client::LabelloApi>,
        command: UiCommand,
    ) -> Option<UiCommand> {
        match command {
            UiCommand::ImportCapabilities { request } => self.spawn_import_message(async move {
                UiMessage::ImportCapabilitiesLoaded {
                    request,
                    result: api
                        .import_capabilities()
                        .await
                        .map_err(|error| error.to_string()),
                }
            }),
            UiCommand::CreateImport {
                request,
                body,
                idempotency_key,
            } => self.spawn_import_message(async move {
                UiMessage::ImportJobLoaded {
                    request,
                    result: Box::new(
                        api.create_import(body, &idempotency_key)
                            .await
                            .map_err(|error| error.to_string()),
                    ),
                }
            }),
            UiCommand::GetImport { request, import_id } => self.spawn_import_message(async move {
                UiMessage::ImportJobLoaded {
                    request,
                    result: Box::new(
                        api.get_import(&import_id)
                            .await
                            .map_err(|error| error.to_string()),
                    ),
                }
            }),
            UiCommand::RegisterImportFiles {
                request,
                import_id,
                body,
                idempotency_key,
            } => self.spawn_import_message(async move {
                UiMessage::ImportFilesRegistered {
                    request,
                    result: api
                        .register_import_files(&import_id, body, &idempotency_key)
                        .await
                        .map_err(|error| error.to_string()),
                }
            }),
            UiCommand::BrowseImportRoot {
                request,
                root_id,
                body,
            } => self.spawn_import_message(async move {
                UiMessage::ImportSourceBrowsed {
                    request,
                    result: api
                        .browse_server_import_root(&root_id, body)
                        .await
                        .map_err(|error| error.to_string()),
                }
            }),
            UiCommand::BrowseImportSource {
                request,
                import_id,
                body,
            } => self.spawn_import_message(async move {
                UiMessage::ImportSourceBrowsed {
                    request,
                    result: api
                        .browse_import_source(&import_id, body)
                        .await
                        .map_err(|error| error.to_string()),
                }
            }),
            UiCommand::InspectYoloDescriptor {
                request,
                import_id,
                descriptor_file_id,
                body,
            } => self.spawn_import_message(async move {
                UiMessage::YoloDescriptorInspected {
                    request,
                    descriptor_file_id,
                    result: api
                        .inspect_yolo_descriptor(&import_id, body)
                        .await
                        .map_err(|error| error.to_string()),
                }
            }),
            UiCommand::SealImport {
                request,
                import_id,
                body,
                idempotency_key,
            } => self.spawn_import_message(async move {
                UiMessage::ImportSealed {
                    request,
                    result: api
                        .seal_import(&import_id, body, &idempotency_key)
                        .await
                        .map_err(|error| error.to_string()),
                }
            }),
            UiCommand::PreflightImport {
                request,
                import_id,
                body,
                idempotency_key,
            } => self.spawn_import_message(async move {
                UiMessage::ImportJobLoaded {
                    request,
                    result: Box::new(
                        api.preflight_import(&import_id, body, &idempotency_key)
                            .await
                            .map_err(|error| error.to_string()),
                    ),
                }
            }),
            UiCommand::UpdateImportPlan {
                request,
                import_id,
                body,
                idempotency_key,
            } => self.spawn_import_message(async move {
                UiMessage::ImportPlanUpdated {
                    request,
                    result: Box::new(
                        api.update_import_plan(&import_id, body, &idempotency_key)
                            .await
                            .map_err(|error| error.to_string()),
                    ),
                }
            }),
            UiCommand::ImportDiagnostics {
                request,
                import_id,
                query,
            } => self.spawn_import_message(async move {
                UiMessage::ImportDiagnosticsLoaded {
                    request,
                    result: api
                        .import_diagnostics(&import_id, query)
                        .await
                        .map_err(|error| error.to_string()),
                }
            }),
            UiCommand::CommitImport {
                request,
                import_id,
                body,
                idempotency_key,
            } => self.spawn_import_message(async move {
                UiMessage::ImportCommitted {
                    request,
                    result: api
                        .commit_import(&import_id, body, &idempotency_key)
                        .await
                        .map_err(|error| error.to_string()),
                }
            }),
            UiCommand::CancelImport {
                request,
                import_id,
                idempotency_key,
            } => self.spawn_import_message(async move {
                UiMessage::ImportCancelled {
                    request,
                    result: api
                        .cancel_import(
                            &import_id,
                            labello_client::CancelImportRequest {
                                reason: Some("cancelled by administrator".to_string()),
                            },
                            &idempotency_key,
                        )
                        .await
                        .map_err(|error| error.to_string()),
                }
            }),
            command => return Some(command),
        }
        None
    }
}
