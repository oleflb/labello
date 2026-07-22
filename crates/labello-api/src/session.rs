use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use labello_domain::{Timestamp, UserAccount, UserId, now};
use serde::{Deserialize, Serialize};

use crate::error::{ApiError, ApiResult};

pub const SESSION_COOKIE: &str = "labello_session";
pub const OAUTH_FLOW_COOKIE: &str = "labello_oauth_flow";
const SESSION_LIFETIME_DAYS: i64 = 30;
const OAUTH_STATE_LIFETIME_MINUTES: i64 = 10;

#[derive(Clone)]
pub(crate) struct ServerStore {
    path: Arc<PathBuf>,
    data: Arc<Mutex<StoreData>>,
    load_error: Arc<Option<String>>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct StoreData {
    users: BTreeMap<UserId, UserAccount>,
    sessions: BTreeMap<String, SessionRecord>,
    oauth_flows: BTreeMap<String, OAuthFlowRecord>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionRecord {
    user_id: UserId,
    expires_at: Timestamp,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OAuthFlowRecord {
    cookie_hash: String,
    return_to: String,
    expires_at: Timestamp,
}

pub(crate) struct OAuthFlow {
    pub state: String,
    pub cookie_token: String,
}

impl ServerStore {
    pub(crate) fn new(datasets_root: &Path) -> Self {
        let path = datasets_root.join(".labello-server").join("auth.json");
        let loaded = if path.exists() {
            fs::read(&path)
                .map_err(|error| error.to_string())
                .and_then(|bytes| serde_json::from_slice(&bytes).map_err(|error| error.to_string()))
        } else {
            Ok(StoreData::default())
        };
        let (data, load_error) = match loaded {
            Ok(data) => (data, None),
            Err(error) => {
                tracing::error!(
                    event = "auth.store.load_failed",
                    error_kind = "invalid_or_unreadable",
                    "could not load authentication store"
                );
                (StoreData::default(), Some(error))
            }
        };
        Self {
            path: Arc::new(path),
            data: Arc::new(Mutex::new(data)),
            load_error: Arc::new(load_error),
        }
    }

    pub(crate) fn upsert_user(&self, mut account: UserAccount) -> ApiResult<UserAccount> {
        self.ensure_loaded()?;
        let mut data = self.lock();
        if let Some(existing) = data.users.get(&account.user_id) {
            account.created_at = existing.created_at;
        }
        account.updated_at = now();
        data.users.insert(account.user_id.clone(), account.clone());
        self.save(&data)?;
        Ok(account)
    }

    pub(crate) fn users(&self) -> ApiResult<Vec<UserAccount>> {
        self.ensure_loaded()?;
        Ok(self.lock().users.values().cloned().collect())
    }

    pub(crate) fn user(&self, user_id: &UserId) -> ApiResult<Option<UserAccount>> {
        self.ensure_loaded()?;
        Ok(self.lock().users.get(user_id).cloned())
    }

    pub(crate) fn create_session(&self, user_id: UserId) -> ApiResult<String> {
        self.ensure_loaded()?;
        let token = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let mut data = self.lock();
        prune(&mut data);
        data.sessions.insert(
            token_hash(&token),
            SessionRecord {
                user_id,
                expires_at: now() + chrono::Duration::days(SESSION_LIFETIME_DAYS),
            },
        );
        self.save(&data)?;
        Ok(token)
    }

    pub(crate) fn session_user(&self, token: &str) -> ApiResult<Option<UserAccount>> {
        self.ensure_loaded()?;
        let mut data = self.lock();
        let before = data.sessions.len() + data.oauth_flows.len();
        prune(&mut data);
        let user = data
            .sessions
            .get(&token_hash(token))
            .and_then(|session| data.users.get(&session.user_id))
            .cloned();
        if before != data.sessions.len() + data.oauth_flows.len() {
            self.save(&data)?;
        }
        Ok(user)
    }

    pub(crate) fn delete_session(&self, token: &str) -> ApiResult<()> {
        self.ensure_loaded()?;
        let mut data = self.lock();
        data.sessions.remove(&token_hash(token));
        self.save(&data)
    }

    pub(crate) fn create_oauth_flow(&self, return_to: String) -> ApiResult<OAuthFlow> {
        self.ensure_loaded()?;
        let state = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let cookie_token = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let mut data = self.lock();
        prune(&mut data);
        data.oauth_flows.insert(
            token_hash(&state),
            OAuthFlowRecord {
                cookie_hash: token_hash(&cookie_token),
                return_to,
                expires_at: now() + chrono::Duration::minutes(OAUTH_STATE_LIFETIME_MINUTES),
            },
        );
        self.save(&data)?;
        Ok(OAuthFlow {
            state,
            cookie_token,
        })
    }

    pub(crate) fn consume_oauth_flow(
        &self,
        state: &str,
        cookie_token: &str,
    ) -> ApiResult<Option<String>> {
        self.ensure_loaded()?;
        let mut data = self.lock();
        prune(&mut data);
        let state_hash = token_hash(state);
        let matches_cookie = data
            .oauth_flows
            .get(&state_hash)
            .is_some_and(|flow| flow.cookie_hash == token_hash(cookie_token));
        let return_to = matches_cookie
            .then(|| data.oauth_flows.remove(&state_hash))
            .flatten()
            .map(|flow| flow.return_to);
        self.save(&data)?;
        Ok(return_to)
    }

    fn ensure_loaded(&self) -> ApiResult<()> {
        match self.load_error.as_ref() {
            Some(error) => Err(ApiError::Internal(format!(
                "cannot load server auth store {}: {error}",
                self.path.display()
            ))),
            None => Ok(()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, StoreData> {
        self.data.lock().unwrap_or_else(|poisoned| {
            tracing::error!(
                event = "auth.store.lock_poisoned",
                "authentication store lock recovered after panic"
            );
            poisoned.into_inner()
        })
    }

    fn save(&self, data: &StoreData) -> ApiResult<()> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| ApiError::Internal("auth store has no parent directory".to_string()))?;
        fs::create_dir_all(parent).map_err(store_error)?;
        let temporary = self.path.with_extension("json.tmp");
        fs::write(&temporary, serde_json::to_vec_pretty(data)?).map_err(store_error)?;
        fs::rename(&temporary, self.path.as_ref()).map_err(store_error)
    }
}

fn prune(data: &mut StoreData) {
    let timestamp = now();
    data.sessions
        .retain(|_, session| session.expires_at > timestamp);
    data.oauth_flows
        .retain(|_, flow| flow.expires_at > timestamp);
}

fn token_hash(token: &str) -> String {
    blake3::hash(token.as_bytes()).to_hex().to_string()
}

fn store_error(error: std::io::Error) -> ApiError {
    ApiError::Internal(format!("cannot persist server auth store: {error}"))
}

pub(crate) fn session_cookie(token: &str, secure: bool) -> String {
    format!(
        "{SESSION_COOKIE}={token}; Path=/; HttpOnly; {}; Max-Age={}{}",
        same_site(secure),
        SESSION_LIFETIME_DAYS * 24 * 60 * 60,
        secure_attribute(secure),
    )
}

pub(crate) fn expired_session_cookie(secure: bool) -> String {
    format!(
        "{SESSION_COOKIE}=; Path=/; HttpOnly; {}; Max-Age=0{}",
        same_site(secure),
        secure_attribute(secure),
    )
}

pub(crate) fn oauth_flow_cookie(token: &str, secure: bool) -> String {
    format!(
        "{OAUTH_FLOW_COOKIE}={token}; Path=/auth/github; HttpOnly; {}; Max-Age={}{}",
        same_site(secure),
        OAUTH_STATE_LIFETIME_MINUTES * 60,
        secure_attribute(secure),
    )
}

pub(crate) fn expired_oauth_flow_cookie(secure: bool) -> String {
    format!(
        "{OAUTH_FLOW_COOKIE}=; Path=/auth/github; HttpOnly; {}; Max-Age=0{}",
        same_site(secure),
        secure_attribute(secure),
    )
}

fn same_site(secure: bool) -> &'static str {
    if secure {
        "SameSite=None"
    } else {
        "SameSite=Lax"
    }
}

fn secure_attribute(secure: bool) -> &'static str {
    if secure { "; Secure" } else { "" }
}
