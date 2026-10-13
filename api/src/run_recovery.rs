use crate::background_work::{BackgroundLoop, BackgroundWork, Cadence, Readiness};
use crate::{persistence::unix_now, run_attempt_effects::settle_attempt_mutation, state::AppState};
use scope_postgres::error::PostgresErrorKind;
use std::time::Duration;
use tokio::time::MissedTickBehavior;

const RECOVERY_BATCH_SIZE: u64 = 100;
const RECOVERY_INTERVAL: Duration = Duration::from_secs(15);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct AttemptRecoveryReport {
    pub(crate) requeued: usize,
    pub(crate) lost: usize,
}

pub(crate) async fn reconcile_expired_attempts(
    state: &AppState,
    now_unix: u64,
) -> Result<AttemptRecoveryReport, scope_postgres::error::PostgresError> {
    let attempt_ids = state
        .metadata
        .runs()
        .expired_attempt_ids(now_unix, RECOVERY_BATCH_SIZE)
        .await?;
    let mut report = AttemptRecoveryReport::default();
    for attempt_id in attempt_ids {
        match state
            .metadata
            .runs()
            .expire_attempt(&attempt_id, now_unix)
            .await
        {
            Ok(mutation) => {
                settle_attempt_mutation(state, &mutation).await;
                if mutation.claim.run.state == scope_domain::runs::run::RunState::Queued {
                    report.requeued += 1;
                } else {
                    report.lost += 1;
                }
            }
            Err(error)
                if matches!(
                    error.kind,
                    PostgresErrorKind::Conflict | PostgresErrorKind::NotFound
                ) =>
            {
                tracing::debug!(
                    attempt_id,
                    "run attempt changed while lease recovery was reconciling it"
                );
            }
            Err(error) => return Err(error),
        }
    }
    Ok(report)
}

impl BackgroundWork {
    pub(crate) fn start_run_attempt_recovery(&mut self) {
        self.spawn(
            BackgroundLoop {
                name: "run attempt lease recovery",
                cadence: Cadence::Fixed {
                    period: RECOVERY_INTERVAL,
                    missed_ticks: MissedTickBehavior::Burst,
                },
                readiness: Readiness::Ungated,
            },
            |state| async move {
                let report = reconcile_expired_attempts(&state, unix_now()?).await?;
                if report != AttemptRecoveryReport::default() {
                    tracing::info!(
                        requeued = report.requeued,
                        lost = report.lost,
                        "reconciled expired run attempt leases"
                    );
                }
                Ok(())
            },
        );
    }
}
