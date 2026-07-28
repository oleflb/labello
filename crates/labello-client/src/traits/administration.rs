pub trait StatsApi {
    fn dataset_stats<'a>(&'a self, dataset_id: &'a DatasetId) -> ApiFuture<'a, DatasetStats>;
}

pub trait KeybindingApi {
    fn get_keybindings<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        user_id: &'a UserId,
    ) -> ApiFuture<'a, KeybindingSet>;

    fn save_keybindings<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        keybindings: KeybindingSet,
    ) -> ApiFuture<'a, KeybindingSet>;
}

pub trait PrelabelApi {
    fn list_prelabel_configs<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
    ) -> ApiFuture<'a, Vec<PrelabelConfig>>;

    fn add_prelabel_config<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        config: PrelabelConfig,
    ) -> ApiFuture<'a, PrelabelConfig>;

    fn prelabel_suggestions<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        request: PrelabelSuggestionRequest,
    ) -> ApiFuture<'a, Vec<PrelabelSuggestion>>;
}

pub trait AuthApi {
    fn csrf_token(&self) -> Option<String> {
        None
    }

    fn auth_options<'a>(&'a self) -> ApiFuture<'a, AuthOptions> {
        Box::pin(async {
            Ok(AuthOptions {
                github_oauth: false,
                local_admin_login: false,
            })
        })
    }
    fn local_admin_login<'a>(&'a self) -> ApiFuture<'a, SessionInfo> {
        Box::pin(async {
            Err(ClientError::Api {
                status: 401,
                message: "local administrator login is not available".to_string(),
            })
        })
    }
    fn github_login_url<'a>(&'a self, request: OAuthLoginRequest) -> ApiFuture<'a, String>;
    fn github_callback<'a>(&'a self, request: OAuthCallbackRequest) -> ApiFuture<'a, UserAccount>;
    fn me<'a>(&'a self) -> ApiFuture<'a, SessionInfo> {
        Box::pin(async {
            Err(ClientError::Demo(
                "current session lookup is not implemented by this client".to_string(),
            ))
        })
    }
    fn logout<'a>(&'a self) -> ApiFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
}

pub trait UserApi {
    fn list_dataset_users<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
    ) -> ApiFuture<'a, Vec<DatasetUser>>;
    fn set_dataset_roles<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        request: SetDatasetRolesRequest,
    ) -> ApiFuture<'a, DatasetUser>;
}
