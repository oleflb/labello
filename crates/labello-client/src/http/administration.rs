impl StatsApi for HttpLabelloApi {
    fn dataset_stats<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
    ) -> crate::ApiFuture<'a, DatasetStats> {
        Box::pin(async move {
            Self::json(
                self.request(Method::GET, &format!("/datasets/{dataset_id}/stats"))?
                    .timeout(STATS_REQUEST_TIMEOUT)
                    .send()
                    .await?,
            )
            .await
        })
    }
}

impl KeybindingApi for HttpLabelloApi {
    fn get_keybindings<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        _user_id: &'a UserId,
    ) -> crate::ApiFuture<'a, KeybindingSet> {
        Box::pin(async move {
            Self::json(
                self.request(Method::GET, &format!("/datasets/{dataset_id}/keybindings"))?
                    .send()
                    .await?,
            )
            .await
        })
    }

    fn save_keybindings<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        keybindings: KeybindingSet,
    ) -> crate::ApiFuture<'a, KeybindingSet> {
        Box::pin(async move {
            Self::send_json(
                self.request(Method::PUT, &format!("/datasets/{dataset_id}/keybindings"))?,
                &keybindings,
            )
            .await
        })
    }
}

impl PrelabelApi for HttpLabelloApi {
    fn list_prelabel_configs<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
    ) -> crate::ApiFuture<'a, Vec<PrelabelConfig>> {
        Box::pin(async move {
            Self::json(
                self.request(Method::GET, &format!("/datasets/{dataset_id}/prelabels"))?
                    .send()
                    .await?,
            )
            .await
        })
    }

    fn add_prelabel_config<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        config: PrelabelConfig,
    ) -> crate::ApiFuture<'a, PrelabelConfig> {
        Box::pin(async move {
            Self::send_json(
                self.request(Method::POST, &format!("/datasets/{dataset_id}/prelabels"))?,
                &config,
            )
            .await
        })
    }

    fn prelabel_suggestions<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        request: PrelabelSuggestionRequest,
    ) -> crate::ApiFuture<'a, Vec<PrelabelSuggestion>> {
        Box::pin(async move {
            Self::send_json(
                self.request(
                    Method::POST,
                    &format!("/datasets/{dataset_id}/prelabel-suggestions"),
                )?,
                &request,
            )
            .await
        })
    }
}

impl UserApi for HttpLabelloApi {
    fn list_dataset_users<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
    ) -> crate::ApiFuture<'a, Vec<DatasetUser>> {
        Box::pin(async move {
            Self::json(
                self.request(Method::GET, &format!("/datasets/{dataset_id}/users"))?
                    .send()
                    .await?,
            )
            .await
        })
    }

    fn set_dataset_roles<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        request: SetDatasetRolesRequest,
    ) -> crate::ApiFuture<'a, DatasetUser> {
        Box::pin(async move {
            Self::send_json(
                self.request(Method::PUT, &format!("/datasets/{dataset_id}/roles"))?,
                &request,
            )
            .await
        })
    }
}
