impl StatsApi for DemoLabelloApi {
    fn dataset_stats<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
    ) -> crate::ApiFuture<'a, DatasetStats> {
        Box::pin(async move { Ok(DatasetStats::default()) })
    }
}

impl KeybindingApi for DemoLabelloApi {
    fn get_keybindings<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        user_id: &'a UserId,
    ) -> crate::ApiFuture<'a, KeybindingSet> {
        Box::pin(async move {
            let mut keybindings = self
                .state
                .borrow()
                .keybindings
                .get(user_id)
                .cloned()
                .unwrap_or_else(|| KeybindingSet::defaults_for(user_id.clone()));
            keybindings.normalize();
            Ok(keybindings)
        })
    }

    fn save_keybindings<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        mut keybindings: KeybindingSet,
    ) -> crate::ApiFuture<'a, KeybindingSet> {
        Box::pin(async move {
            keybindings
                .validate()
                .map_err(|error| ClientError::Demo(error.to_string()))?;
            keybindings.normalize();
            self.state
                .borrow_mut()
                .keybindings
                .insert(keybindings.user_id.clone(), keybindings.clone());
            Ok(keybindings)
        })
    }
}

impl PrelabelApi for DemoLabelloApi {
    fn list_prelabel_configs<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
    ) -> crate::ApiFuture<'a, Vec<PrelabelConfig>> {
        Box::pin(async move { Ok(self.dataset(dataset_id)?.prelabel_configs) })
    }

    fn add_prelabel_config<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        config: PrelabelConfig,
    ) -> crate::ApiFuture<'a, PrelabelConfig> {
        Box::pin(async move {
            let mut state = self.state.borrow_mut();
            let dataset = state
                .datasets
                .get_mut(dataset_id)
                .ok_or_else(|| ClientError::Demo(format!("dataset {dataset_id} does not exist")))?;
            dataset
                .prelabel_configs
                .retain(|existing| existing.config_id != config.config_id);
            dataset.prelabel_configs.push(config.clone());
            Ok(config)
        })
    }

    fn prelabel_suggestions<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        _request: PrelabelSuggestionRequest,
    ) -> crate::ApiFuture<'a, Vec<PrelabelSuggestion>> {
        Box::pin(async move { Ok(Vec::new()) })
    }
}

impl UserApi for DemoLabelloApi {
    fn list_dataset_users<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
    ) -> crate::ApiFuture<'a, Vec<DatasetUser>> {
        Box::pin(async move {
            let metadata = self.dataset(dataset_id)?;
            Ok(metadata
                .role_assignments
                .into_iter()
                .map(|assignment| DatasetUser {
                    account: UserAccount {
                        user_id: assignment.user_id.clone(),
                        display_name: assignment.user_id.to_string(),
                        github_user_id: None,
                        github_login: None,
                        created_at: assignment.assigned_at,
                        updated_at: assignment.assigned_at,
                    },
                    roles: assignment.roles.into_iter().collect(),
                })
                .collect())
        })
    }

    fn set_dataset_roles<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        request: SetDatasetRolesRequest,
    ) -> crate::ApiFuture<'a, DatasetUser> {
        Box::pin(async move {
            let mut state = self.state.borrow_mut();
            let metadata = state
                .datasets
                .get_mut(dataset_id)
                .ok_or_else(|| ClientError::Demo(format!("dataset {dataset_id} does not exist")))?;
            metadata
                .role_assignments
                .retain(|assignment| assignment.user_id != request.user_id);
            if !request.roles.is_empty() {
                metadata
                    .role_assignments
                    .push(labello_domain::DatasetRoleAssignment {
                        dataset_id: dataset_id.clone(),
                        user_id: request.user_id.clone(),
                        roles: request.roles.iter().cloned().collect(),
                        assigned_at: labello_domain::now(),
                        assigned_by: Some(UserId::from("demo_user")),
                    });
            }
            let timestamp = labello_domain::now();
            Ok(DatasetUser {
                account: UserAccount {
                    user_id: request.user_id.clone(),
                    display_name: request.user_id.to_string(),
                    github_user_id: None,
                    github_login: None,
                    created_at: timestamp,
                    updated_at: timestamp,
                },
                roles: request.roles,
            })
        })
    }
}
