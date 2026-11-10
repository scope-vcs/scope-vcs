use crate::{
    config::{LOCAL_APP_ORIGIN, SCOPE_APP_ORIGIN_ENV, non_empty_env},
    error::ApiError,
};

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
