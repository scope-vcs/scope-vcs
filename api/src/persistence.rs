use crate::error::ApiError;
use std::{fs, path::Path};

#[cfg(test)]
pub(crate) fn test_data_dir() -> std::sync::Arc<tempfile::TempDir> {
    std::sync::Arc::new(
        tempfile::Builder::new()
            .prefix("scope-vcs-test-data-")
            .tempdir()
            .expect("test data directory must be creatable"),
    )
}

pub(crate) fn ensure_private_dir(path: &Path) -> Result<(), ApiError> {
    fs::create_dir_all(path).map_err(ApiError::internal)?;
    let metadata = fs::symlink_metadata(path).map_err(ApiError::internal)?;
    if !metadata.file_type().is_dir() {
        return Err(ApiError::internal_message(format!(
            "{} is not a directory",
            path.display()
        )));
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = metadata.permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(path, permissions).map_err(ApiError::internal)?;
        let mode = fs::symlink_metadata(path)
            .map_err(ApiError::internal)?
            .permissions()
            .mode()
            & 0o777;
        if mode != 0o700 {
            return Err(ApiError::internal_message(format!(
                "{} must be private to serve Git projections",
                path.display()
            )));
        }
    }

    Ok(())
}

pub(crate) fn unix_now() -> Result<u64, ApiError> {
    scope_service_runtime::unix_now().map_err(ApiError::internal)
}
