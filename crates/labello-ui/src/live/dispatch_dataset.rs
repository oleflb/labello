impl LabelloApp {
    fn dispatch_dataset_command(
        &self,
        api: Rc<dyn labello_client::LabelloApi>,
        command: UiCommand,
    ) -> Option<UiCommand> {
        match command {
            UiCommand::DatasetList { request } => self.spawn_message(request.clone(), async move {
                UiMessage::DatasetList {
                    request,
                    result: api.list_datasets().await.map_err(|error| error.to_string()),
                }
            }),
            UiCommand::CreateDataset {
                request,
                dataset_id,
                name,
                admin_user_id,
            } => self.spawn_message(request.clone(), async move {
                UiMessage::DatasetCreated {
                    request,
                    result: Box::new(
                        api.create_dataset(labello_client::CreateDatasetRequest {
                            dataset_id,
                            name,
                            admin_user_id,
                        })
                        .await
                        .map_err(|error| error.to_string()),
                    ),
                }
            }),
            UiCommand::LoadDataset {
                request,
                dataset_id,
                user_id,
            } => self.spawn_message(request.clone(), async move {
                let result = async {
                    let metadata = api.get_dataset(&dataset_id).await?;
                    let keybindings = api.get_keybindings(&dataset_id, &user_id).await?;
                    Ok::<_, labello_client::ClientError>(LoadedDataset {
                        metadata,
                        keybindings,
                    })
                }
                .await
                .map_err(|error| error.to_string());
                UiMessage::DatasetLoaded {
                    request,
                    result: Box::new(result),
                }
            }),
            UiCommand::LoadAdmin {
                request,
                dataset_id,
            } => self.spawn_message(request.clone(), async move {
                let result = async {
                    let metadata = api.get_admin_dataset(&dataset_id).await?;
                    let users = api.list_dataset_users(&dataset_id).await?;
                    Ok::<_, labello_client::ClientError>(LoadedAdmin { metadata, users })
                }
                .await
                .map_err(|error| error.to_string());
                UiMessage::AdminLoaded {
                    request,
                    result: Box::new(result),
                }
            }),
            UiCommand::SaveAdmin { request, metadata } => {
                let dataset_id = metadata.dataset_id.clone();
                let update = UpdateDatasetConfigRequest::from_metadata(&metadata);
                self.spawn_message(request.clone(), async move {
                    UiMessage::AdminSaved {
                        request,
                        result: Box::new(
                            api.update_dataset_config(&dataset_id, update)
                                .await
                                .map_err(|error| error.to_string()),
                        ),
                    }
                });
            }
            UiCommand::SaveDatasetRoles {
                request,
                dataset_id,
                user_id,
                roles,
            } => self.spawn_message(request.clone(), async move {
                UiMessage::DatasetRolesSaved {
                    request,
                    result: api
                        .set_dataset_roles(&dataset_id, SetDatasetRolesRequest { user_id, roles })
                        .await
                        .map_err(|error| error.to_string()),
                }
            }),
            UiCommand::LoadImages {
                request,
                dataset_id,
                query,
            } => self.spawn_message(request.clone(), async move {
                UiMessage::ImagesLoaded {
                    request,
                    result: api
                        .list_images(&dataset_id, query)
                        .await
                        .map_err(|error| error.to_string()),
                }
            }),
            UiCommand::LoadSnapshots {
                request,
                dataset_id,
            } => self.spawn_message(request.clone(), async move {
                UiMessage::SnapshotsLoaded {
                    request,
                    result: api
                        .list_snapshots(&dataset_id)
                        .await
                        .map_err(|error| error.to_string()),
                }
            }),
            UiCommand::CreateSnapshot {
                request,
                dataset_id,
            } => self.spawn_message(request.clone(), async move {
                UiMessage::SnapshotCreated {
                    request,
                    result: api
                        .create_snapshot(&dataset_id)
                        .await
                        .map_err(|error| error.to_string()),
                }
            }),
            UiCommand::DownloadSnapshot {
                request,
                dataset_id,
                snapshot_id,
                path,
            } => self.spawn_message(request.clone(), async move {
                UiMessage::SnapshotDownloaded {
                    request,
                    result: api
                        .get_snapshot_file(&dataset_id, &snapshot_id, &path)
                        .await
                        .map_err(|error| error.to_string()),
                }
            }),
            UiCommand::Ingest {
                request,
                dataset_id,
            } => self.spawn_message(request.clone(), async move {
                UiMessage::IngestJobLoaded {
                    request,
                    result: api
                        .start_ingest_job(&dataset_id)
                        .await
                        .map_err(|error| error.to_string()),
                }
            }),
            UiCommand::PollIngest {
                request,
                dataset_id,
                job_id,
            } => self.spawn_message(request.clone(), async move {
                UiMessage::IngestJobLoaded {
                    request,
                    result: api
                        .get_ingest_job(&dataset_id, &job_id)
                        .await
                        .map_err(|error| error.to_string()),
                }
            }),
            command => return Some(command),
        }
        None
    }
}
