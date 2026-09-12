use crate::{
    auth::{clerk::bearer_token, tokens::machine_token_hash},
    error::ApiError,
    persistence::unix_now,
    state::AppState,
};
use axum::http::HeaderMap;
use scope_postgres::db::DispatchClaim;

pub(crate) async fn require_attempt(
    state: &AppState,
    headers: &HeaderMap,
    attempt_id: &str,
) -> Result<DispatchClaim, ApiError> {
    let secret = machine_secret(headers, "scope_attempt_", "attempt")?;
    Ok(state
        .metadata
        .runs()
        .authenticate_attempt(attempt_id, &machine_token_hash(secret), unix_now()?)
        .await?)
}

pub(crate) fn attempt_token_hash(headers: &HeaderMap) -> Result<String, ApiError> {
    Ok(machine_token_hash(machine_secret(
        headers,
        "scope_attempt_",
        "attempt",
    )?))
}

pub(crate) fn bootstrap_token_hash(headers: &HeaderMap) -> Result<String, ApiError> {
    Ok(machine_token_hash(machine_secret(
        headers,
        "scope_bootstrap_",
        "runtime bootstrap",
    )?))
}

fn machine_secret<'a>(
    headers: &'a HeaderMap,
    prefix: &str,
    label: &str,
) -> Result<&'a str, ApiError> {
    let secret = bearer_token(headers)?
        .ok_or_else(|| ApiError::unauthorized(format!("{label} token required")))?;
    if !secret.starts_with(prefix) {
        return Err(ApiError::unauthorized(format!(
            "{label} credentials are invalid"
        )));
    }
    Ok(secret)
}
