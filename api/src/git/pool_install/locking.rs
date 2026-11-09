use crate::error::ApiError;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, Weak},
    time::Duration,
};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

pub(crate) async fn write_shelf<T: Send + 'static>(
    path: &Path,
    write: impl FnOnce() -> Result<T, ApiError> + Send + 'static,
) -> Result<T, ApiError> {
    let guard = lock_shelf(path).await?;
    crate::git::blocking::run(move || {
        let _guard = guard;
        write()
    })
    .await
}

async fn lock_shelf(path: &Path) -> Result<OwnedMutexGuard<()>, ApiError> {
    let lock = {
        type Locks = BTreeMap<PathBuf, Weak<AsyncMutex<()>>>;
        static LOCKS: OnceLock<Mutex<Locks>> = OnceLock::new();
        let mut locks = LOCKS
            .get_or_init(Mutex::default)
            .lock()
            .map_err(|_| ApiError::internal_message("fetch shelf locks poisoned"))?;
        locks.retain(|_, lock| lock.strong_count() > 0);
        let lock = locks.entry(path.to_path_buf()).or_default();
        match lock.upgrade() {
            Some(lock) => lock,
            None => {
                let created = Arc::new(AsyncMutex::new(()));
                *lock = Arc::downgrade(&created);
                created
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(2), lock.lock_owned())
        .await
        .map_err(|_| ApiError::infrastructure_unavailable("Git object shelf is busy; retry fetch"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn a_busy_shelf_times_out_without_blocking_other_shelves() {
        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join("first.git");
        let held = lock_shelf(&first).await.unwrap();
        let other = lock_shelf(&directory.path().join("other.git"))
            .await
            .unwrap();
        let error = lock_shelf(&first).await.unwrap_err();
        assert!(error.operator_diagnostic().contains("retry fetch"));
        drop(held);
        assert!(lock_shelf(&first).await.is_ok());
        drop(other);
    }
}
