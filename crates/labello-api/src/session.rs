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
    oauth_states: BTreeMap<String, Timestamp>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionRecord {
    user_id: UserId,
    expires_at: Timestamp,
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
            Err(error) => (StoreData::default(), Some(error)),
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
        let before = data.sessions.len() + data.oauth_states.len();
        prune(&mut data);
        let user = data
            .sessions
            .get(&token_hash(token))
            .and_then(|session| data.users.get(&session.user_id))
            .cloned();
        if before != data.sessions.len() + data.oauth_states.len() {
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

    pub(crate) fn create_oauth_state(&self) -> ApiResult<String> {
        self.ensure_loaded()?;
        let state = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let mut data = self.lock();
        prune(&mut data);
        data.oauth_states.insert(
            token_hash(&state),
            now() + chrono::Duration::minutes(OAUTH_STATE_LIFETIME_MINUTES),
        );
        self.save(&data)?;
        Ok(state)
    }

    pub(crate) fn consume_oauth_state(&self, state: &str) -> ApiResult<bool> {
        self.ensure_loaded()?;
        let mut data = self.lock();
        prune(&mut data);
        let valid = data.oauth_states.remove(&token_hash(state)).is_some();
        self.save(&data)?;
        Ok(valid)
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
        self.data
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
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
    data.oauth_states
        .retain(|_, expires_at| *expires_at > timestamp);
}

fn token_hash(token: &str) -> String {
    blake3::hash(token.as_bytes()).to_hex().to_string()
}

fn store_error(error: std::io::Error) -> ApiError {
    ApiError::Internal(format!("cannot persist server auth store: {error}"))
}

pub(crate) fn session_cookie(token: &str, secure: bool) -> String {
    format!(
        "{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={};{}",
        SESSION_LIFETIME_DAYS * 24 * 60 * 60,
        if secure { " Secure" } else { "" }
    )
}

pub(crate) fn expired_session_cookie(secure: bool) -> String {
    format!(
        "{SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0;{}",
        if secure { " Secure" } else { "" }
    )
}
