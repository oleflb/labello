fn import_actor(state: &ApiState, headers: &HeaderMap) -> ApiResult<labello_domain::Actor> {
    actor_from_headers(state, headers)
}

fn ensure_import_admin(state: &ApiState, user_id: &UserId) -> ApiResult<()> {
    if state.is_bootstrap_admin(user_id) {
        Ok(())
    } else {
        Err(ApiError::Forbidden(
            "bootstrap administrator access required".to_string(),
        ))
    }
}

fn require_service(state: &ApiState) -> ApiResult<&storage::ImportService> {
    state
        .import_service()
        .map(AsRef::as_ref)
        .ok_or_else(|| ApiError::Conflict("dataset import is unavailable".to_string()))
}

async fn require_owned_job(
    state: &ApiState,
    import_id: &ImportId,
    owner: &UserId,
) -> ApiResult<storage::ImportJob> {
    require_service(state)?
        .job(import_id, owner)
        .await
        .map_err(map_storage)
}

fn idempotency_key(headers: &HeaderMap) -> ApiResult<&str> {
    let key = required_header(headers, IDEMPOTENCY_HEADER)?;
    if key.len() > 200
        || key.is_empty()
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && byte != b',' && byte != b';')
    {
        return Err(ApiError::BadRequest(
            "idempotency-key must be 1-200 visible ASCII characters".to_string(),
        ));
    }
    Ok(key)
}

fn required_header<'a>(headers: &'a HeaderMap, name: &str) -> ApiResult<&'a str> {
    let values = headers.get_all(name);
    let mut values = values.iter();
    let value = values
        .next()
        .filter(|_| values.next().is_none())
        .ok_or_else(|| ApiError::BadRequest(format!("exactly one {name} header is required")))?;
    value
        .to_str()
        .map_err(|_| ApiError::BadRequest(format!("{name} header is invalid")))
}

fn required_u64_header(headers: &HeaderMap, name: &str) -> ApiResult<u64> {
    required_header(headers, name)?
        .parse()
        .map_err(|_| ApiError::BadRequest(format!("{name} must be an unsigned integer")))
}

fn parse_digest(value: &str) -> ApiResult<&str> {
    let value = value.strip_prefix("blake3=").unwrap_or(value);
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ApiError::BadRequest(
            "digest must contain a full hexadecimal BLAKE3 digest".to_string(),
        ));
    }
    Ok(value)
}
