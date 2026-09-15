use super::*;
use crate::execution::fake::FakeEcs;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

#[tokio::test]
async fn ambiguous_cleanup_does_not_block_new_dispatch_or_cancellation_batches() {
    let provider = FakeEcs::new().await;
    provider.starts.add_permits(2);
    let mut phases = CloudReconciliation::default();
    let cleanup = provider.client.clone();
    phases.cleanup.start_if_idle("cleanup", async move {
        cleanup.stop_terminal_task("uncertain").await?;
        Ok(1)
    });
    provider.wait_for("stop", 1).await;

    let completed = Arc::new(AtomicUsize::new(0));
    for round in 1..=2 {
        let dispatched = completed.clone();
        let client = provider.client.clone();
        phases.dispatch.start_if_idle("dispatch", async move {
            client
                .start(&format!("attempt_{round}"), "token")
                .await
                .map_err(|error| anyhow::anyhow!("{error:?}"))?;
            dispatched.fetch_add(1, Ordering::SeqCst);
            Ok(1)
        });
        let client = provider.client.clone();
        let canceled = completed.clone();
        phases
            .cancellation
            .start_if_idle("cancellation", async move {
                client.stop_terminal_task("canceled").await?;
                canceled.fetch_add(1, Ordering::SeqCst);
                Ok(1)
            });
        tokio::time::timeout(Duration::from_secs(5), async {
            phases.dispatch.0.join_next().await.unwrap().unwrap();
            phases.cancellation.0.join_next().await.unwrap().unwrap();
        })
        .await
        .expect("healthy phases finish while cleanup is pending");
        assert_eq!(completed.load(Ordering::SeqCst), round * 2);
        assert_eq!(provider.count("start"), round);
        assert_eq!(provider.count("stop"), round + 1);
        assert_eq!(phases.cleanup.0.len(), 1);
    }
    // Repeated control polls do not create duplicate cleanup work.
    for _ in 0..100 {
        phases
            .cleanup
            .start_if_idle("cleanup", async { panic!("duplicate cleanup") });
    }
    assert_eq!(phases.cleanup.0.len(), 1);
    assert!(phases.cleanup.0.try_join_next().is_none());
}

#[tokio::test]
async fn shutdown_cancels_owned_phase_work_instead_of_detaching_it() {
    struct Running(Arc<AtomicUsize>);
    impl Drop for Running {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }
    let active = Arc::new(AtomicUsize::new(0));
    let (started, mut received) = tokio::sync::mpsc::channel(3);
    let mut phases = CloudReconciliation::default();
    for task in [
        &mut phases.cleanup,
        &mut phases.cancellation,
        &mut phases.dispatch,
    ] {
        let active = active.clone();
        let started = started.clone();
        task.start_if_idle("fixture", async move {
            active.fetch_add(1, Ordering::SeqCst);
            let _running = Running(active);
            started.send(()).await.unwrap();
            std::future::pending().await
        });
    }
    for _ in 0..3 {
        received.recv().await.unwrap();
    }
    drop(phases);
    tokio::time::timeout(Duration::from_secs(1), async {
        while active.load(Ordering::SeqCst) != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("all phase futures are dropped on shutdown");
}
