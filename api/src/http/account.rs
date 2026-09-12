use crate::{
    auth::scope::optional_scope_user,
    error::ApiError,
    http::responses::{HealthResponse, ReadinessCheckResponse, ReadinessResponse, user_response},
    state::AppState,
};
use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use scope_api_contract::{AccountSessionResponse, SessionIdentity};
use scope_domain::account::SessionIdentity as DomainSessionIdentity;

pub(crate) async fn healthz() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        service: "api",
    })
}

pub(crate) async fn readyz(State(state): State<AppState>) -> (StatusCode, Json<ReadinessResponse>) {
    let database_ready = state.metadata.admin().readiness_check().await.is_ok();
    let object_store_ready = state.object_store.readiness_check().is_ok();
    let ready = database_ready && object_store_ready;

    (
        if ready {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        },
        Json(ReadinessResponse {
            status: if ready { "ok" } else { "unavailable" },
            service: "api",
            checks: vec![
                ReadinessCheckResponse {
                    name: "database",
                    status: if database_ready { "ok" } else { "unavailable" },
                },
                ReadinessCheckResponse {
                    name: "object_store",
                    status: if object_store_ready {
                        "ok"
                    } else {
                        "unavailable"
                    },
                },
            ],
        }),
    )
}

pub(crate) async fn get_account_session(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<AccountSessionResponse>, ApiError> {
    let user = optional_scope_user(&state, &headers).await?;
    Ok(Json(AccountSessionResponse {
        identity: user
            .as_ref()
            .map(|user| SessionIdentity::from(DomainSessionIdentity::from(user))),
        user: user.map(user_response),
    }))
}
