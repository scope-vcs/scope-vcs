use crate::{
    error::ApiError,
    persistence::unix_now,
    state::AppState,
    use_cases::{
        request_auto_merge,
        request_checks::{self, RequestCheckRecoveryCursor},
    },
};
use std::{future::Future, sync::Arc, time::Duration};
use tokio::{
    sync::{Notify, watch},
    task::JoinHandle,
};

const AUTO_MERGE_INTERVAL: Duration = Duration::from_secs(5);
const CHECK_RECOVERY_INTERVAL: Duration = Duration::from_secs(30);

pub struct ReconcilerRuntime {
    name: &'static str,
    stop: watch::Sender<bool>,
    task: JoinHandle<()>,
}

impl ReconcilerRuntime {
    pub fn stop_signal(&self) -> watch::Sender<bool> {
        self.stop.clone()
    }

    pub async fn shutdown(self) {
        let _ = self.stop.send(true);
        if let Err(error) = self.task.await {
            tracing::error!(reconciler = self.name, %error, "reconciler exited unexpectedly");
        }
    }
}

impl AppState {
    pub fn start_request_auto_merge_runtime(&self) -> ReconcilerRuntime {
        self.start_reconciler(
            "request auto-merge",
            AUTO_MERGE_INTERVAL,
            Some(self.auto_merge_wakeup.clone()),
            (),
            |state, ()| async move {
                let result =
                    async { request_auto_merge::reconcile_once(&state, unix_now()?).await }
                        .await
                        .map(drop);
                ((), result)
            },
        )
    }

    pub fn start_request_checks_runtime(&self) -> ReconcilerRuntime {
        self.start_reconciler(
            "request check recovery",
            CHECK_RECOVERY_INTERVAL,
            None,
            RequestCheckRecoveryCursor::default(),
            |state, mut cursor| async move {
                let result = request_checks::reconcile_request_checks_once(&state, &mut cursor)
                    .await
                    .map(drop);
                (cursor, result)
            },
        )
    }

    fn start_reconciler<P, F, Fut>(
        &self,
        name: &'static str,
        interval: Duration,
        wakeup: Option<Arc<Notify>>,
        initial: P,
        pass: F,
    ) -> ReconcilerRuntime
    where
        P: Clone + Send + 'static,
        F: Fn(AppState, P) -> Fut + Send + 'static,
        Fut: Future<Output = (P, Result<(), ApiError>)> + Send + 'static,
    {
        let state = self.clone();
        let (stop, mut stopped) = watch::channel(false);
        let task = tokio::spawn(async move {
            let mut progress = initial;
            loop {
                if *stopped.borrow() {
                    break;
                }
                let readiness_state = state.clone();
                let unchanged = progress.clone();
                let attempt = pass(state.clone(), progress.clone());
                match tokio::spawn(async move {
                    match readiness_state.metadata.admin().readiness_check().await {
                        Ok(()) => attempt.await,
                        Err(error) => (unchanged, Err(error.into())),
                    }
                })
                .await
                {
                    Ok((next, result)) => {
                        progress = next;
                        if let Err(error) = result {
                            tracing::warn!(
                                reconciler = name,
                                error = %error.operator_diagnostic(),
                                "reconciliation failed; retrying"
                            );
                        }
                    }
                    Err(error) => tracing::error!(
                        reconciler = name,
                        %error,
                        "reconciliation pass exited unexpectedly; retrying"
                    ),
                }
                let woken = async {
                    match &wakeup {
                        Some(wakeup) => wakeup.notified().await,
                        None => std::future::pending().await,
                    }
                };
                tokio::select! {
                    _ = stopped.changed() => break,
                    _ = woken => {},
                    _ = tokio::time::sleep(interval) => {},
                }
            }
        });
        ReconcilerRuntime { name, stop, task }
    }
}
