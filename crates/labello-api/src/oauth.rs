use labello_domain::{UserAccount, UserId, now};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::error::{ApiError, ApiResult};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubOAuthConfig {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
}

impl GithubOAuthConfig {
    pub fn authorization_url(&self, state: &str) -> ApiResult<String> {
        let mut url = Url::parse("https://github.com/login/oauth/authorize")
            .map_err(|error| ApiError::BadRequest(error.to_string()))?;
        url.query_pairs_mut()
            .append_pair("client_id", &self.client_id)
            .append_pair("redirect_uri", &self.redirect_uri)
            .append_pair("scope", "read:user user:email")
            .append_pair("state", state);
        Ok(url.to_string())
    }
}

#[derive(Debug, Deserialize)]
struct GithubTokenResponse {
    access_token: String,
}

#[derive(Debug, Deserialize)]
struct GithubUserResponse {
    id: u64,
    login: String,
    name: Option<String>,
}

pub async fn exchange_code(
    client: &reqwest::Client,
    config: &GithubOAuthConfig,
    code: &str,
) -> ApiResult<UserAccount> {
    let token: GithubTokenResponse = client
        .post("https://github.com/login/oauth/access_token")
        .header("Accept", "application/json")
        .form(&serde_json::json!({
            "client_id": config.client_id,
            "client_secret": config.client_secret,
            "code": code,
            "redirect_uri": config.redirect_uri,
        }))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let user: GithubUserResponse = client
        .get("https://api.github.com/user")
        .bearer_auth(token.access_token)
        .header("User-Agent", "labello")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let timestamp = now();
    Ok(UserAccount {
        user_id: UserId::from(format!("github_{}", user.id)),
        display_name: user.name.unwrap_or_else(|| user.login.clone()),
        github_user_id: Some(user.id.to_string()),
        github_login: Some(user.login),
        created_at: timestamp,
        updated_at: timestamp,
    })
}
