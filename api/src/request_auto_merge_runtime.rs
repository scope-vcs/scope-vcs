//! Process lifetime for durable request auto-merge reconciliation.

use crate::{persistence::unix_now, state::AppState, use_cases::request_auto_merge};
use std::time::Duration;
use tokio::{sync::watch, task::JoinHandle};

const POLL_INTERVAL: Duration = Duration::from_secs(5);

pub struct RequestAutoMergeRuntime {
    stop: watch::Sender<bool>,
    task: JoinHandle<()>,
}

impl RequestAutoMergeRuntime {
    pub fn stop_signal(&self) -> watch::Sender<bool> {
        self.stop.clone()
    }

    /// Finish any merge already being prepared before releasing process resources.
    pub async fn shutdown(self) {
        let _ = self.stop.send(true);
        if let Err(error) = self.task.await {
            tracing::error!(%error, "request auto-merge reconciler exited unexpectedly");
        }
    }
}

impl AppState {
    pub fn start_request_auto_merge_runtime(&self) -> RequestAutoMergeRuntime {
        let state = self.clone();
        let (stop, mut stopped) = watch::channel(false);
        let task = tokio::spawn(async move {
            loop {
                if *stopped.borrow() {
                    break;
                }
                // Isolate a pass so a panicking adapter cannot silently end reconciliation.
                // Its claims remain recoverable after their persisted leases expire.
                let pass_state = state.clone();
                match tokio::spawn(async move {
                    pass_state.metadata.admin().readiness_check().await?;
                    request_auto_merge::reconcile_once(&pass_state, unix_now()?).await
                })
                .await
                {
                    Ok(Ok(_)) => {}
                    Ok(Err(error)) => tracing::warn!(
                        error = %error.operator_diagnostic(),
                        "request auto-merge reconciliation failed; retrying"
                    ),
                    Err(error) => tracing::error!(
                        %error,
                        "request auto-merge pass exited unexpectedly; retrying"
                    ),
                }
                tokio::select! {
                    _ = stopped.changed() => break,
                    _ = state.auto_merge_wakeup.notified() => {},
                    _ = tokio::time::sleep(POLL_INTERVAL) => {},
                }
            }
        });
        RequestAutoMergeRuntime { stop, task }
    }
}
