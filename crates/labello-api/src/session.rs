use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::Write,
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

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct StoreData {
    users: BTreeMap<UserId, UserAccount>,
    sessions: BTreeMap<String, SessionRecord>,
    oauth_flows: BTreeMap<String, OAuthFlowRecord>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionRecord {
    user_id: UserId,
    expires_at: Timestamp,
    #[serde(default)]
    csrf_token: String,
}

#[derive(Clone, Serialize, Deserialize)]
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

pub(crate) struct SessionTokens {
    pub cookie: String,
    pub csrf: String,
}

pub(crate) struct AuthenticatedSession {
    pub account: UserAccount,
    pub csrf_token: String,
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
        let (mut data, mut load_error) = match loaded {
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
        let mut migrated = false;
        for session in data.sessions.values_mut() {
            if session.csrf_token.is_empty() {
                session.csrf_token = random_token();
                migrated = true;
            }
        }
        if migrated && let Err(error) = save_store(&path, &data) {
            tracing::error!(
                event = "auth.store.migration_failed",
                error_kind = "persistence",
                "could not migrate authentication store"
            );
            load_error = Some(error.to_string());
        }
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

    pub(crate) fn create_session(&self, user_id: UserId) -> ApiResult<SessionTokens> {
        self.ensure_loaded()?;
        let cookie = random_token();
        let csrf = random_token();
        let mut data = self.lock();
        prune(&mut data);
        data.sessions.insert(
            token_hash(&cookie),
            SessionRecord {
                user_id,
                expires_at: now() + chrono::Duration::days(SESSION_LIFETIME_DAYS),
                csrf_token: csrf.clone(),
            },
        );
        self.save(&data)?;
        Ok(SessionTokens { cookie, csrf })
    }

    pub(crate) fn session(&self, token: &str) -> ApiResult<Option<AuthenticatedSession>> {
        self.ensure_loaded()?;
        let mut data = self.lock();
        let before = data.sessions.len() + data.oauth_flows.len();
        prune(&mut data);
        let session = data.sessions.get(&token_hash(token)).and_then(|session| {
            data.users
                .get(&session.user_id)
                .cloned()
                .map(|account| AuthenticatedSession {
                    account,
                    csrf_token: session.csrf_token.clone(),
                })
        });
        if before != data.sessions.len() + data.oauth_flows.len() {
            self.save(&data)?;
        }
        Ok(session)
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
        save_store(&self.path, data)
    }
}

fn save_store(path: &Path, data: &StoreData) -> ApiResult<()> {
    let parent = path
        .parent()
        .ok_or_else(|| ApiError::Internal("auth store has no parent directory".to_string()))?;
    fs::create_dir_all(parent).map_err(store_error)?;
    restrict_directory(parent)?;
    let temporary = path.with_extension("json.tmp");
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary).map_err(store_error)?;
    restrict_file(&temporary)?;
    file.write_all(&serde_json::to_vec_pretty(data)?)
        .map_err(store_error)?;
    fs::rename(&temporary, path).map_err(store_error)
}

fn prune(data: &mut StoreData) {
    let timestamp = now();
    data.sessions
        .retain(|_, session| session.expires_at > timestamp && !session.csrf_token.is_empty());
    data.oauth_flows
        .retain(|_, flow| flow.expires_at > timestamp);
}

fn token_hash(token: &str) -> String {
    blake3::hash(token.as_bytes()).to_hex().to_string()
}

fn random_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

#[cfg(unix)]
fn restrict_directory(path: &Path) -> ApiResult<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(store_error)
}

#[cfg(not(unix))]
fn restrict_directory(_path: &Path) -> ApiResult<()> {
    Ok(())
}

#[cfg(unix)]
fn restrict_file(path: &Path) -> ApiResult<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(store_error)
}

#[cfg(not(unix))]
fn restrict_file(_path: &Path) -> ApiResult<()> {
    Ok(())
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
