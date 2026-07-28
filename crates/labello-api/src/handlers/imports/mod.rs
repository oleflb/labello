use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Path as AxumPath, Query, State},
    http::HeaderMap,
    routing::{get, post},
};
use labello_client as client;
use labello_domain::{ImportId, UserId};
use labello_storage as storage;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{
    ApiState,
    auth::actor_from_headers,
    error::{ApiError, ApiResult},
};

const IDEMPOTENCY_HEADER: &str = "idempotency-key";
const MAX_JSON_BODY: usize = 1024 * 1024;
const MAX_REGISTRATION_BODY: usize = 8 * 1024 * 1024;

include!("routes.rs");
include!("policy.rs");
include!("adapters.rs");
include!("control.rs");
include!("tests.rs");
