use crate::error::ApiError;

/// The closure owns every lease, permit and temporary resource needed by its
/// work; cancellation of the awaiting task does not cancel a blocking child.
pub(crate) async fn run<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, ApiError> + Send + 'static,
) -> Result<T, ApiError> {
    tokio::task::spawn_blocking(work).await.map_err(|error| {
        ApiError::internal_message(format!("Git blocking operation failed: {error}"))
    })?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
            mpsc,
        },
        time::Duration,
    };

    struct Lease(Arc<AtomicBool>);
    impl Drop for Lease {
        fn drop(&mut self) {
            self.0.store(false, Ordering::SeqCst);
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn blocking_work_leaves_runtime_responsive_and_retains_owner_after_abort() {
        let alive = Arc::new(AtomicBool::new(true));
        let lease = Lease(alive.clone());
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let work = tokio::spawn(run(move || {
            let _lease = lease;
            started_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            Ok(())
        }));
        tokio::time::timeout(Duration::from_secs(2), started_rx)
            .await
            .unwrap()
            .unwrap();
        tokio::time::sleep(Duration::from_millis(1)).await;
        work.abort();
        assert!(work.await.unwrap_err().is_cancelled());
        assert!(alive.load(Ordering::SeqCst));
        release_tx.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while alive.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }
}
