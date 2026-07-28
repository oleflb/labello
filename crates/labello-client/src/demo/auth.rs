impl AuthApi for DemoLabelloApi {
    fn auth_options<'a>(&'a self) -> crate::ApiFuture<'a, AuthOptions> {
        Box::pin(async move {
            Ok(AuthOptions {
                github_oauth: true,
                local_admin_login: false,
            })
        })
    }

    fn local_admin_login<'a>(&'a self) -> crate::ApiFuture<'a, SessionInfo> {
        Box::pin(async move {
            Err(ClientError::Api {
                status: 401,
                message: "local administrator login is not available in demo mode".to_string(),
            })
        })
    }

    fn github_login_url<'a>(&'a self, _request: OAuthLoginRequest) -> crate::ApiFuture<'a, String> {
        Box::pin(async move { Ok("https://github.com/login/oauth/authorize".to_string()) })
    }

    fn github_callback<'a>(
        &'a self,
        _request: OAuthCallbackRequest,
    ) -> crate::ApiFuture<'a, UserAccount> {
        Box::pin(async move {
            let timestamp = labello_domain::now();
            Ok(UserAccount {
                user_id: UserId::from("demo_user"),
                display_name: "Demo User".to_string(),
                github_user_id: None,
                github_login: None,
                created_at: timestamp,
                updated_at: timestamp,
            })
        })
    }

    fn me<'a>(&'a self) -> crate::ApiFuture<'a, SessionInfo> {
        Box::pin(async move {
            let account = self
                .github_callback(OAuthCallbackRequest {
                    code: String::new(),
                    state: String::new(),
                })
                .await?;
            Ok(SessionInfo {
                account,
                can_create_datasets: true,
                csrf_token: "demo-csrf-token".to_string(),
            })
        })
    }

    fn logout<'a>(&'a self) -> crate::ApiFuture<'a, ()> {
        Box::pin(async move { Ok(()) })
    }
}
