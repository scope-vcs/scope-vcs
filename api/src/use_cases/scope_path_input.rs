use crate::error::ApiError;
use scope_domain::policy::ScopePath;

/// Normalizes a caller-supplied file path into an absolute, non-root `ScopePath`.
pub(crate) fn normalized_scope_path(path: &str) -> Result<ScopePath, ApiError> {
    let scope_path = ScopePath::parse(format!("/{}", path.trim_start_matches('/')))
        .map_err(ApiError::bad_request)?;
    if scope_path == ScopePath::root() {
        return Err(ApiError::bad_request("file path is required"));
    }
    Ok(scope_path)
}
