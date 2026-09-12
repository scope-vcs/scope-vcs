use crate::error::ApiError;
use std::{fs, path::Path};

#[cfg(test)]
pub(crate) fn test_data_dir() -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("test clock must be after UNIX epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "scope-vcs-test-data-{}-{nanos}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ))
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
