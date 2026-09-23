use crate::{
    auth::{clerk::bearer_token, tokens::machine_token_hash},
    error::ApiError,
    persistence::unix_now,
    state::AppState,
};
use axum::{Json, extract::State, http::HeaderMap};
use hmac::{Hmac, KeyInit, Mac};
use scope_domain::runs::dispatch_authorization::DispatchAuthorization;
use serde::Deserialize;
use sha2::Sha256;

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "lowercase", deny_unknown_fields)]
pub(crate) enum DispatchRequest {
    Start {
        attempt_id: String,
        bootstrap_token: String,
    },
    Stop {
        attempt_id: String,
    },
}

pub(crate) async fn authorize(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<DispatchRequest>,
) -> Result<Json<DispatchAuthorization>, ApiError> {
    require_broker(state.dispatch_broker_token.as_deref(), &headers)?;
    let runs = state.metadata.runs();
    let authorization = match input {
        DispatchRequest::Start {
            attempt_id,
            bootstrap_token,
        } => {
            if !bootstrap_token.starts_with("scope_bootstrap_") {
                return Err(ApiError::unauthorized(
                    "invalid runtime bootstrap credentials",
                ));
            }
            runs.authorize_dispatch_start(
                &attempt_id,
                &machine_token_hash(&bootstrap_token),
                unix_now()?,
            )
            .await?
        }
        DispatchRequest::Stop { attempt_id } => runs.authorize_dispatch_stop(&attempt_id).await?,
    };
    Ok(Json(authorization))
}

fn require_broker(expected: Option<&str>, headers: &HeaderMap) -> Result<(), ApiError> {
    let expected =
        expected.ok_or_else(|| ApiError::unauthorized("dispatch authorization is disabled"))?;
    let actual =
        bearer_token(headers)?.ok_or_else(|| ApiError::unauthorized("broker token required"))?;
    let mut actual_mac = Hmac::<Sha256>::new_from_slice(b"scope-dispatch-broker-auth")
        .map_err(ApiError::internal)?;
    let mut expected_mac = actual_mac.clone();
    expected_mac.update(expected.as_bytes());
    actual_mac.update(actual.as_bytes());
    actual_mac
        .verify_slice(&expected_mac.finalize().into_bytes())
        .map_err(|_| ApiError::unauthorized("invalid broker credentials"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderValue, header::AUTHORIZATION};

    #[test]
    fn broker_authentication_fails_closed() {
        let mut headers = HeaderMap::new();
        assert!(require_broker(None, &headers).is_err());
        assert!(require_broker(Some("broker-secret"), &headers).is_err());
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_static("Bearer wrong-secret"),
        );
        assert!(require_broker(Some("broker-secret"), &headers).is_err());
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_static("Bearer broker-secret"),
        );
        assert!(require_broker(None, &headers).is_err());
        assert!(require_broker(Some("broker-secret"), &headers).is_ok());
    }

    #[test]
    fn requests_reject_dispatch_configuration_and_task_arns() {
        for input in [
            r#"{"action":"start","attempt_id":"a","bootstrap_token":"b","image":"attacker"}"#,
            r#"{"action":"start","attempt_id":"a","bootstrap_token":"b","execution_role":"attacker"}"#,
            r#"{"action":"start","attempt_id":"a","bootstrap_token":"b","secret_arn":"attacker"}"#,
            r#"{"action":"start","attempt_id":"a","bootstrap_token":"b","subnets":["attacker"]}"#,
            r#"{"action":"start","attempt_id":"a","bootstrap_token":"b","cpu":8192}"#,
            r#"{"action":"stop","attempt_id":"a","task_arn":"attacker"}"#,
            r#"{"action":"stop","attempt_id":"a","bootstrap_token":"b"}"#,
        ] {
            assert!(serde_json::from_str::<DispatchRequest>(input).is_err());
        }
    }
}
