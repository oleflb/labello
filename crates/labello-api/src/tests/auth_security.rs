#[tokio::test]
async fn auth_options_are_public_and_only_advertise_capabilities() {
    let temp = tempfile::tempdir().unwrap();
    let app = router(
        ApiState::new(temp.path())
            .with_local_admin_login(Some(labello_domain::UserId::from("local_admin")))
            .with_github_oauth(crate::GithubOAuthConfig {
                client_id: "client-id".to_string(),
                client_secret: "oauth-secret".to_string(),
                redirect_uri: "https://api.example.com/auth/github/callback".to_string(),
            }),
    );

    let response = app
        .oneshot(
            Request::builder()
                .uri("/auth/options")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let options: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        options,
        json!({ "githubOauth": true, "localAdminLogin": true })
    );
    let body = String::from_utf8(body.to_vec()).unwrap();
    assert!(!body.contains("oauth-secret"));
    assert!(!body.contains("local_admin"));
}

#[tokio::test]
async fn local_admin_login_is_not_found_when_disabled() {
    let temp = tempfile::tempdir().unwrap();
    let response = router(ApiState::new(temp.path()))
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/local-admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn local_admin_login_creates_session_and_requires_configured_browser_origin() {
    let temp = tempfile::tempdir().unwrap();
    let app = router(
        ApiState::new(temp.path())
            .with_browser_origins(vec!["https://app.example.com".to_string()])
            .unwrap()
            .with_session_cookie_secure(false)
            .with_bootstrap_admins([labello_domain::UserId::from("bootstrap_admin")])
            .with_local_admin_login(Some(labello_domain::UserId::from("bootstrap_admin"))),
    );

    let rejected = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/local-admin")
                .header(header::ORIGIN, "https://other.example.com")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::UNAUTHORIZED);

    let login = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/local-admin")
                .header(header::ORIGIN, "https://app.example.com")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(login.status(), StatusCode::OK);
    let cookie = login.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .to_string();
    assert!(cookie.starts_with("labello_session="));
    assert!(cookie.contains("HttpOnly"));
    assert!(cookie.contains("SameSite=Lax"));
    assert!(!cookie.contains("; Secure"));
    let body = to_bytes(login.into_body(), usize::MAX).await.unwrap();
    let session: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(session["account"]["userId"], "bootstrap_admin");
    assert_eq!(session["account"]["displayName"], "bootstrap_admin");
    assert!(session["account"]["githubUserId"].is_null());
    assert!(session["account"]["githubLogin"].is_null());
    assert_eq!(session["canCreateDatasets"], true);
    let csrf_token = session["csrfToken"].as_str().unwrap().to_string();
    assert_eq!(csrf_token.len(), 64);

    let me = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/me")
                .header(header::COOKIE, cookie.split(';').next().unwrap())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(me.status(), StatusCode::OK);
    assert_eq!(me.headers()[header::CACHE_CONTROL], "no-store");
    let body = to_bytes(me.into_body(), usize::MAX).await.unwrap();
    let session: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(session["account"]["userId"], "bootstrap_admin");
    assert_eq!(session["canCreateDatasets"], true);
    assert_eq!(session["csrfToken"], csrf_token);

    let rotated = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/local-admin")
                .header(header::ORIGIN, "https://app.example.com")
                .header(header::COOKIE, cookie.split(';').next().unwrap())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rotated.status(), StatusCode::OK);
    let rotated_cookie = rotated.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .to_string();
    assert_ne!(
        rotated_cookie.split(';').next().unwrap(),
        cookie.split(';').next().unwrap()
    );
    let body = to_bytes(rotated.into_body(), usize::MAX).await.unwrap();
    let rotated_session: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_ne!(rotated_session["csrfToken"], csrf_token);

    let old_session = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/me")
                .header(header::COOKIE, cookie.split(';').next().unwrap())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(old_session.status(), StatusCode::UNAUTHORIZED);

    let missing_origin = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/local-admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing_origin.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn unsafe_session_requests_require_csrf_and_validate_optional_origin() {
    let temp = tempfile::tempdir().unwrap();
    let state = ApiState::new(temp.path())
        .with_browser_origins(vec!["https://app.example.com".to_string()])
        .unwrap();
    let timestamp = now();
    state
        .server_store
        .upsert_user(UserAccount {
            user_id: UserId::from("admin"),
            display_name: "Admin".to_string(),
            github_user_id: None,
            github_login: None,
            created_at: timestamp,
            updated_at: timestamp,
        })
        .unwrap();
    let session = state.create_session(UserId::from("admin")).unwrap();
    let app = production_router(state);
    let request = |dataset_id: &str, csrf: Option<&str>, origin: Option<&str>| {
        let mut builder = Request::builder()
            .method("POST")
            .uri("/datasets")
            .header(
                header::COOKIE,
                format!("labello_session={}", session.cookie),
            )
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(csrf) = csrf {
            builder = builder.header(crate::csrf::HEADER, csrf);
        }
        if let Some(origin) = origin {
            builder = builder.header(header::ORIGIN, origin);
        }
        builder
            .body(Body::from(
                json!({
                    "datasetId": dataset_id,
                    "name": "CSRF dataset",
                    "adminUserId": "admin"
                })
                .to_string(),
            ))
            .unwrap()
    };

    let missing = app
        .clone()
        .oneshot(request("missing", None, None))
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);

    let wrong = app
        .clone()
        .oneshot(request("wrong", Some("wrong-token"), None))
        .await
        .unwrap();
    assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);

    let wrong_origin = app
        .clone()
        .oneshot(request(
            "wrong-origin",
            Some(&session.csrf),
            Some("https://other.example.com"),
        ))
        .await
        .unwrap();
    assert_eq!(wrong_origin.status(), StatusCode::UNAUTHORIZED);

    let browser = app
        .clone()
        .oneshot(request(
            "browser-origin",
            Some(&session.csrf),
            Some("https://app.example.com"),
        ))
        .await
        .unwrap();
    assert_eq!(browser.status(), StatusCode::OK);

    let native = app
        .oneshot(request("native-no-origin", Some(&session.csrf), None))
        .await
        .unwrap();
    assert_eq!(native.status(), StatusCode::OK);
}

#[tokio::test]
async fn development_headers_do_not_authenticate() {
    let temp = tempfile::tempdir().unwrap();
    let response = production_router(ApiState::new(temp.path()))
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/datasets")
                .header(axum::http::HeaderName::from_static("x-user-id"), "admin")
                .header("x-dev-token", "secret")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "datasetId": "ds",
                        "name": "Dataset",
                        "adminUserId": "admin"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn review_and_adjudication_actor_ids_must_match_caller() {
    let temp = tempfile::tempdir().unwrap();
    let app = router(ApiState::new(temp.path()));
    create_dataset(&app).await;
    let timestamp = labello_domain::now().to_rfc3339();

    let review = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/datasets/ds/images/img_1/reviews?assignmentId=asg_1&imageId=img_1&taskId=task_1&kind=review")
                .header("x-test-user-id", "admin")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "reviewId": "rev_1",
                        "target": {
                            "targetType": "task",
                            "task_id": "task_1"
                        },
                        "reviewerUserId": "someone_else",
                        "decision": "approved",
                        "timestamp": timestamp.clone(),
                        "comment": null
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let review_status = review.status();
    let review_body = to_bytes(review.into_body(), usize::MAX).await.unwrap();
    assert_eq!(
        review_status,
        StatusCode::UNAUTHORIZED,
        "{}",
        String::from_utf8_lossy(&review_body)
    );

    let adjudication = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/datasets/ds/images/img_1/adjudications?assignmentId=asg_1&imageId=img_1&taskId=task_1&kind=adjudication")
                .header("x-test-user-id", "admin")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "adjudicationId": "adj_1",
                        "taskId": "task_1",
                        "annotationIds": [],
                        "adjudicatorUserId": "someone_else",
                        "decision": "accept_annotation",
                        "resolution": "accepted",
                        "timestamp": timestamp
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(adjudication.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn session_survives_state_recreation_and_logout_invalidates_it() {
    let temp = tempfile::tempdir().unwrap();
    let state = ApiState::new(temp.path()).with_session_cookie_secure(false);
    let timestamp = labello_domain::now();
    state
        .server_store
        .upsert_user(labello_domain::UserAccount {
            user_id: labello_domain::UserId::from("session_user"),
            display_name: "Session User".to_string(),
            github_user_id: Some("42".to_string()),
            github_login: Some("session-user".to_string()),
            created_at: timestamp,
            updated_at: timestamp,
        })
        .unwrap();
    let session_tokens = state
        .create_session(labello_domain::UserId::from("session_user"))
        .unwrap();
    let token = session_tokens.cookie;
    let csrf_token = session_tokens.csrf;
    let auth_store =
        std::fs::read_to_string(temp.path().join(".labello-server/auth.json")).unwrap();
    assert!(!auth_store.contains(&token));
    assert!(auth_store.contains(&csrf_token));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(temp.path().join(".labello-server"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(temp.path().join(".labello-server/auth.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    let app = production_router(ApiState::new(temp.path()).with_session_cookie_secure(false));
    let me = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/me")
                .header(header::COOKIE, format!("labello_session={token}"))
                .header(axum::http::HeaderName::from_static("x-user-id"), "spoofed")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(me.status(), StatusCode::OK);
    assert_eq!(me.headers()[header::CACHE_CONTROL], "no-store");
    let body = to_bytes(me.into_body(), usize::MAX).await.unwrap();
    let session: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(session["account"]["userId"], "session_user");
    assert_eq!(session["canCreateDatasets"], false);
    assert_eq!(session["csrfToken"], csrf_token);

    let missing_csrf = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/logout")
                .header(header::COOKIE, format!("labello_session={token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing_csrf.status(), StatusCode::UNAUTHORIZED);

    let wrong_csrf = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/logout")
                .header(header::COOKIE, format!("labello_session={token}"))
                .header(crate::csrf::HEADER, "wrong-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(wrong_csrf.status(), StatusCode::UNAUTHORIZED);

    let still_active = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/me")
                .header(header::COOKIE, format!("labello_session={token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(still_active.status(), StatusCode::OK);

    let logout = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/logout")
                .header(header::COOKIE, format!("labello_session={token}"))
                .header(crate::csrf::HEADER, &csrf_token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(logout.status(), StatusCode::NO_CONTENT);
    let cleared = logout.headers()[header::SET_COOKIE].to_str().unwrap();
    assert!(cleared.contains("HttpOnly"));
    assert!(cleared.contains("SameSite=Lax"));
    assert!(!cleared.contains("Secure"));
    assert!(cleared.contains("Max-Age=0"));

    let expired = app
        .oneshot(
            Request::builder()
                .uri("/me")
                .header(header::COOKIE, format!("labello_session={token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(expired.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn legacy_sessions_receive_a_persisted_csrf_token_on_load() {
    let temp = tempfile::tempdir().unwrap();
    let state = ApiState::new(temp.path());
    let timestamp = now();
    state
        .server_store
        .upsert_user(UserAccount {
            user_id: UserId::from("legacy_user"),
            display_name: "Legacy User".to_string(),
            github_user_id: None,
            github_login: None,
            created_at: timestamp,
            updated_at: timestamp,
        })
        .unwrap();
    let session = state.create_session(UserId::from("legacy_user")).unwrap();
    let path = temp.path().join(".labello-server/auth.json");
    let mut stored: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    for record in stored["sessions"].as_object_mut().unwrap().values_mut() {
        record.as_object_mut().unwrap().remove("csrfToken");
    }
    std::fs::write(&path, serde_json::to_vec_pretty(&stored).unwrap()).unwrap();
    drop(state);

    let app = production_router(ApiState::new(temp.path()));
    let me = app
        .oneshot(
            Request::builder()
                .uri("/me")
                .header(
                    header::COOKIE,
                    format!("labello_session={}", session.cookie),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(me.status(), StatusCode::OK);
    let body = to_bytes(me.into_body(), usize::MAX).await.unwrap();
    let migrated: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let csrf = migrated["csrfToken"].as_str().unwrap();
    assert_eq!(csrf.len(), 64);
    let persisted = std::fs::read_to_string(path).unwrap();
    assert!(persisted.contains(csrf));
}

#[tokio::test]
async fn oauth_flow_binds_state_to_browser_and_redirects_once_to_valid_return_target() {
    let temp = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mock_address = listener.local_addr().unwrap();
    let mock_github = axum::Router::new()
        .route(
            "/token",
            axum::routing::post(|| async { axum::Json(json!({ "access_token": "github-token" })) }),
        )
        .route(
            "/user",
            axum::routing::get(|| async {
                axum::Json(json!({ "id": 42, "login": "octocat", "name": "Octo Cat" }))
            }),
        );
    tokio::spawn(async move { axum::serve(listener, mock_github).await.unwrap() });

    let state = ApiState::new(temp.path())
        .with_browser_origins(vec!["https://app.example.com".to_string()])
        .unwrap()
        .with_session_cookie_secure(true)
        .with_github_oauth(crate::GithubOAuthConfig {
            client_id: "client".to_string(),
            client_secret: "secret".to_string(),
            redirect_uri: "https://api.example.com/auth/github/callback".to_string(),
        })
        .with_github_oauth_endpoints(crate::oauth::GithubOAuthEndpoints {
            token_url: format!("http://{mock_address}/token"),
            user_url: format!("http://{mock_address}/user"),
        });
    let dataset_id = labello_domain::DatasetId::from("oauth-dataset");
    state
        .repo(&dataset_id)
        .unwrap()
        .initialize(labello_domain::DatasetMetadata::new(
            dataset_id.clone(),
            "OAuth dataset",
            labello_domain::now(),
        ))
        .await
        .unwrap();
    let app = router(state.clone());
    let return_to = "https://app.example.com/datasets/ds?tab=review";
    let login = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/auth/github/login?returnTo={}",
                    urlencoding::encode(return_to)
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(login.status(), StatusCode::TEMPORARY_REDIRECT);
    let location = login.headers()[header::LOCATION].to_str().unwrap();
    let url = url::Url::parse(location).unwrap();
    let generated = url
        .query_pairs()
        .find_map(|(name, value)| (name == "state").then(|| value.into_owned()))
        .unwrap();
    let browser_a_cookie = login.headers()[header::SET_COOKIE].to_str().unwrap();
    assert!(browser_a_cookie.starts_with("labello_oauth_flow="));
    assert!(browser_a_cookie.contains("Path=/auth/github"));
    assert!(browser_a_cookie.contains("HttpOnly"));
    assert!(browser_a_cookie.contains("SameSite=None"));
    assert!(browser_a_cookie.contains("Secure"));
    let browser_a_cookie = browser_a_cookie.split(';').next().unwrap().to_string();

    let invalid_return = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/auth/github/login?returnTo=https%3A%2F%2Fevil.example%2Fsteal")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(invalid_return.status(), StatusCode::BAD_REQUEST);

    let missing_cookie = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/auth/github/callback?code=unused&state={generated}"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing_cookie.status(), StatusCode::UNAUTHORIZED);

    let browser_b_login = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/auth/github/login?returnTo=https%3A%2F%2Fapp.example.com%2Fother")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let browser_b_cookie = browser_b_login.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();

    let wrong_browser = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/auth/github/callback?code=unused&state={generated}"
                ))
                .header(header::COOKIE, browser_b_cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(wrong_browser.status(), StatusCode::UNAUTHORIZED);

    let callback = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/auth/github/callback?code=valid-code&state={generated}"
                ))
                .header(header::COOKIE, &browser_a_cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(callback.status(), StatusCode::SEE_OTHER);
    assert_eq!(callback.headers()[header::LOCATION], return_to);
    let set_cookies: Vec<_> = callback
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|value| value.to_str().unwrap())
        .collect();
    assert_eq!(set_cookies.len(), 2);
    assert!(set_cookies.iter().any(|cookie| {
        cookie.starts_with("labello_session=")
            && cookie.contains("SameSite=None")
            && cookie.contains("Secure")
    }));
    assert!(set_cookies.iter().any(|cookie| {
        cookie.starts_with("labello_oauth_flow=;") && cookie.contains("Max-Age=0")
    }));
    let metadata = state
        .repo(&dataset_id)
        .unwrap()
        .load_dataset_config()
        .await
        .unwrap();
    let oauth_assignment = metadata
        .role_assignments
        .iter()
        .find(|assignment| assignment.user_id == labello_domain::UserId::from("github_42"))
        .unwrap();
    assert_eq!(
        oauth_assignment.roles,
        std::collections::BTreeSet::from([labello_domain::DatasetRole::Annotator])
    );
    let session_cookie = set_cookies
        .iter()
        .find(|cookie| cookie.starts_with("labello_session="))
        .unwrap()
        .split(';')
        .next()
        .unwrap();
    let me = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/me")
                .header(header::COOKIE, session_cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(me.status(), StatusCode::OK);
    let body = to_bytes(me.into_body(), usize::MAX).await.unwrap();
    let session: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(session["csrfToken"].as_str().unwrap().len(), 64);

    let replay = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/auth/github/callback?code=valid-code&state={generated}"
                ))
                .header(header::COOKIE, browser_a_cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(replay.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn credentialed_cors_only_allows_configured_origins() {
    let temp = tempfile::tempdir().unwrap();
    let app = router(
        ApiState::new(temp.path())
            .with_browser_origins(vec!["https://app.remote.example".to_string()])
            .unwrap(),
    );
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri("/imports")
                .header(header::ORIGIN, "https://app.remote.example")
                .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
                .header(
                    header::ACCESS_CONTROL_REQUEST_HEADERS,
                    "content-type,x-csrf-token,idempotency-key,upload-offset,upload-length,digest",
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN],
        "https://app.remote.example"
    );
    assert_eq!(
        response.headers()[header::ACCESS_CONTROL_ALLOW_CREDENTIALS],
        "true"
    );
    let allowed_headers = response.headers()[header::ACCESS_CONTROL_ALLOW_HEADERS]
        .to_str()
        .unwrap();
    for allowed in [
        "content-type",
        "x-csrf-token",
        "idempotency-key",
        "upload-offset",
        "upload-length",
        "digest",
    ] {
        assert!(
            allowed_headers.split(',').any(|header| header == allowed),
            "missing CORS header {allowed} in {allowed_headers}"
        );
    }

    let actual = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/health")
                .header(header::ORIGIN, "https://app.remote.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        actual.headers()[header::ACCESS_CONTROL_EXPOSE_HEADERS],
        "x-image-width,x-image-height,x-request-id"
    );

    let denied = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .header(header::ORIGIN, "https://evil.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        denied
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .is_none()
    );
}

#[test]
fn browser_origin_configuration_rejects_empty_and_non_origin_urls() {
    let temp = tempfile::tempdir().unwrap();
    assert!(
        ApiState::new(temp.path())
            .with_browser_origins(Vec::new())
            .is_err()
    );
    for invalid in [
        "https://app.example/path",
        "https://app.example?query=yes",
        "file:///tmp/app",
        "not a URL",
    ] {
        assert!(
            ApiState::new(temp.path())
                .with_browser_origins(vec![invalid.to_string()])
                .is_err(),
            "accepted {invalid}"
        );
    }
}

#[tokio::test]
async fn import_mutations_require_session_csrf_and_allowed_browser_origin() {
    let temp = tempfile::tempdir().unwrap();
    let service = labello_storage::ImportService::new(
        temp.path(),
        labello_storage::ImportConfig {
            enabled: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let state = ApiState::new(temp.path())
        .with_browser_origins(vec!["https://app.example.com".to_string()])
        .unwrap()
        .with_import_service(service);
    let timestamp = now();
    state
        .server_store
        .upsert_user(UserAccount {
            user_id: UserId::from("admin"),
            display_name: "Admin".to_string(),
            github_user_id: None,
            github_login: None,
            created_at: timestamp,
            updated_at: timestamp,
        })
        .unwrap();
    let session = state.create_session(UserId::from("admin")).unwrap();
    let app = production_router(state);
    let body = json!({
        "destinationDatasetId": "csrf-import",
        "destinationName": "CSRF import",
        "profile": "coco_instances_gt_v1",
        "source": { "transport": "browser_folder" },
        "attestations": {
            "groundTruth": true, "exhaustive": true,
            "coverageScope": [], "provenance": "fixture"
        }
    })
    .to_string();
    let request = |csrf: Option<&str>, origin: Option<&str>| {
        let mut builder = Request::builder()
            .method("POST")
            .uri("/imports")
            .header(
                header::COOKIE,
                format!("labello_session={}", session.cookie),
            )
            .header("idempotency-key", "csrf-create")
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(csrf) = csrf {
            builder = builder.header(crate::csrf::HEADER, csrf);
        }
        if let Some(origin) = origin {
            builder = builder.header(header::ORIGIN, origin);
        }
        builder.body(Body::from(body.clone())).unwrap()
    };
    let missing = app.clone().oneshot(request(None, None)).await.unwrap();
    assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);
    let wrong_origin = app
        .clone()
        .oneshot(request(
            Some(&session.csrf),
            Some("https://other.example.com"),
        ))
        .await
        .unwrap();
    assert_eq!(wrong_origin.status(), StatusCode::UNAUTHORIZED);
    let accepted = app
        .oneshot(request(
            Some(&session.csrf),
            Some("https://app.example.com"),
        ))
        .await
        .unwrap();
    assert_eq!(accepted.status(), StatusCode::OK);
}
