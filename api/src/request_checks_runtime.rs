use crate::{
    state::AppState,
    use_cases::request_checks::{self, RequestCheckRecoveryCursor},
};
use std::time::Duration;
use tokio::{sync::watch, task::JoinHandle};

const POLL_INTERVAL: Duration = Duration::from_secs(30);

pub struct RequestChecksRuntime {
    stop: watch::Sender<bool>,
    task: JoinHandle<()>,
}

impl RequestChecksRuntime {
    pub fn stop_signal(&self) -> watch::Sender<bool> {
        self.stop.clone()
    }

    pub async fn shutdown(self) {
        let _ = self.stop.send(true);
        if let Err(error) = self.task.await {
            tracing::error!(%error, "request check reconciler exited unexpectedly");
        }
    }
}

impl AppState {
    pub fn start_request_checks_runtime(&self) -> RequestChecksRuntime {
        let state = self.clone();
        let (stop, mut stopped) = watch::channel(false);
        let task = tokio::spawn(async move {
            let mut cursor = RequestCheckRecoveryCursor::default();
            loop {
                if *stopped.borrow() {
                    break;
                }
                let pass_state = state.clone();
                let mut pass_cursor = cursor.clone();
                match tokio::spawn(async move {
                    let result = async {
                        pass_state.metadata.admin().readiness_check().await?;
                        request_checks::reconcile_request_checks_once(&pass_state, &mut pass_cursor)
                            .await
                    }
                    .await;
                    (pass_cursor, result)
                })
                .await
                {
                    Ok((next_cursor, result)) => {
                        cursor = next_cursor;
                        if let Err(error) = result {
                            tracing::warn!(error = %error.operator_diagnostic(), "request check reconciliation failed; retrying");
                        }
                    }
                    Err(error) => {
                        tracing::error!(%error, "request check reconciliation pass exited unexpectedly; retrying")
                    }
                }
                tokio::select! {
                    _ = stopped.changed() => break,
                    _ = tokio::time::sleep(POLL_INTERVAL) => {},
                }
            }
        });
        RequestChecksRuntime { stop, task }
    }
}
