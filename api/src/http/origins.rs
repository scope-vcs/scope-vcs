use crate::{
    config::{LOCAL_APP_ORIGIN, SCOPE_APP_ORIGIN_ENV, non_empty_env},
    error::ApiError,
    state::AppState,
};

pub(crate) fn public_git_origin(state: &AppState) -> &str {
    &state.git_public_url
}

pub(crate) fn public_app_origin(action: &str) -> Result<String, ApiError> {
    non_empty_env(SCOPE_APP_ORIGIN_ENV)
        .or_else(|| cfg!(debug_assertions).then(|| LOCAL_APP_ORIGIN.to_string()))
        .map(|value| value.trim_end_matches('/').to_string())
        .ok_or_else(|| {
            ApiError::infrastructure_unavailable(format!(
                "{SCOPE_APP_ORIGIN_ENV} is required to {action}"
            ))
        })
}
