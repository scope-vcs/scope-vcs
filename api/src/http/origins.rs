use crate::{
    config::{
        LOCAL_API_ORIGIN, LOCAL_APP_ORIGIN, SCOPE_API_PUBLIC_URL_ENV, SCOPE_APP_ORIGIN_ENV,
        non_empty_env,
    },
    error::ApiError,
};
use scope_api_contract::routes;

pub(crate) fn public_app_origin(action: &str) -> Result<String, ApiError> {
    public_origin(SCOPE_APP_ORIGIN_ENV, LOCAL_APP_ORIGIN, action)
}

pub(crate) fn public_mcp_resource() -> Result<String, ApiError> {
    public_origin(SCOPE_API_PUBLIC_URL_ENV, LOCAL_API_ORIGIN, "serve agents")
        .map(|origin| format!("{origin}{}", routes::MCP))
}

fn public_origin(env: &str, local: &str, action: &str) -> Result<String, ApiError> {
    non_empty_env(env)
        .or_else(|| cfg!(debug_assertions).then(|| local.to_string()))
        .map(|value| value.trim_end_matches('/').to_string())
        .ok_or_else(|| {
            ApiError::infrastructure_unavailable(format!("{env} is required to {action}"))
        })
}
