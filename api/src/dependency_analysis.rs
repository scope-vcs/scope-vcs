mod snapshot;

use crate::{
    after_commit::AfterCommit,
    background_work::{BackgroundLoop, BackgroundWork, Cadence, Readiness, StopSignal},
    error::ApiError,
    state::AppState,
};
use scope_dependency_analyzer::{ANALYZE_COMMAND, MAX_OUTPUT_BYTES};
use scope_domain::dependency_analysis::{AnalyzerOutput, DEPENDENCY_ANALYZER_VERSION};
use scope_git_process::{ProcessCancellation, ProcessLimits, run_cancellable};
use scope_postgres::db::{DependencyAnalysisClaim, DependencyCompletion};
use scope_service_runtime::unix_now;
use std::{
    process::Command,
    time::{Duration, Instant},
};
use tracing::Instrument as _;

const LEASE_SECONDS: u64 = 60;
const HEARTBEAT: Duration = Duration::from_secs(5);
const ANALYZER_TIMEOUT: Duration = Duration::from_secs(120);
const ANALYSIS_TIMEOUT: Duration = Duration::from_secs(300);
const BACKFILL_INTERVAL: Duration = Duration::from_secs(5 * 60);
const BACKFILL_BATCH_SIZE: usize = 10;
const POLL_INTERVAL: Duration = Duration::from_secs(1);
const PUBLIC_FAILURE: &str = "Dependency check could not finish. It will retry automatically.";

#[derive(Clone, Copy, PartialEq, Eq)]
enum PollOutcome {
    Idle,
    Worked,
    Shutdown,
}

impl BackgroundWork {
    pub(crate) fn start_dependency_analysis(&mut self) {
        let stop = self.stopping();
        self.spawn_with_progress(
            BackgroundLoop {
                name: "dependency analysis",
                cadence: Cadence::AfterPass {
                    delay: POLL_INTERVAL,
                    wakeup: None,
                },
                readiness: Readiness::SchemaRequired,
            },
            BackfillSchedule::default(),
            move |state, mut backfill| {
                let stop = stop.clone();
                async move {
                    let result = analyze_until_idle(&state, &mut backfill, &stop).await;
                    (
                        backfill,
                        result.map_err(|error| ApiError::internal_message(format!("{error:#}"))),
                    )
                }
            },
        );
    }
}

async fn analyze_until_idle(
    state: &AppState,
    backfill: &mut BackfillSchedule,
    stop: &StopSignal,
) -> anyhow::Result<()> {
    while !stop.is_set() && process_next(state, backfill, stop).await? == PollOutcome::Worked {}
    Ok(())
}

#[derive(Clone, Default)]
struct BackfillSchedule {
    last_started: Option<Instant>,
    more_candidates: bool,
}

impl BackfillSchedule {
    fn is_due(&self, now: Instant) -> bool {
        self.more_candidates
            || self
                .last_started
                .is_none_or(|started| now.duration_since(started) >= BACKFILL_INTERVAL)
    }

    fn record(&mut self, started: Instant, enqueued: u64, batch_size: usize) {
        self.last_started = Some(started);
        self.more_candidates = enqueued >= batch_size as u64 && batch_size > 0;
    }
}

async fn process_next(
    state: &AppState,
    backfill: &mut BackfillSchedule,
    stop: &StopSignal,
) -> anyhow::Result<PollOutcome> {
    let metadata = &state.metadata;
    let now = unix_now()?;
    let started = Instant::now();
    if backfill.is_due(started) {
        let enqueued = metadata
            .jobs()
            .enqueue_dependency_analysis_backfill(
                DEPENDENCY_ANALYZER_VERSION,
                now,
                BACKFILL_BATCH_SIZE,
            )
            .await?;
        backfill.record(started, enqueued, BACKFILL_BATCH_SIZE);
    }
    let Some(claim) = metadata
        .jobs()
        .claim_dependency_analysis(
            state.repo_events.origin_id(),
            DEPENDENCY_ANALYZER_VERSION,
            now,
            LEASE_SECONDS,
        )
        .await?
    else {
        return Ok(PollOutcome::Idle);
    };
    let span = tracing::info_span!(
        parent: None,
        "job.dependency_analysis",
        otel.kind = "consumer",
        scope.job.kind = "dependency_analysis",
        scope.job.id = %claim.lease_generation,
    );
    claim.queued_trace.link(&span);
    analyze_claimed(state, claim, stop.clone())
        .instrument(span)
        .await
}

async fn analyze_claimed(
    state: &AppState,
    claim: DependencyAnalysisClaim,
    mut stop: StopSignal,
) -> anyhow::Result<PollOutcome> {
    let metadata = &state.metadata;
    if claim.reusable_analysis.is_some() {
        let completed = metadata
            .jobs()
            .complete_reused_dependency_analysis_claim(&claim, unix_now()?)
            .await?;
        if completed == DependencyCompletion::Completed {
            publish_change(state, &claim);
        }
        return Ok(PollOutcome::Worked);
    }
    let task_claim = claim.clone();
    let task_state = state.clone();
    let cancellation = ProcessCancellation::new();
    let task_cancellation = cancellation.clone();
    let mut task = tokio::spawn(
        async move { analyze(&task_state, &task_claim, task_cancellation).await }.in_current_span(),
    );
    let deadline = tokio::time::sleep(ANALYSIS_TIMEOUT);
    tokio::pin!(deadline);
    let mut interval = tokio::time::interval(HEARTBEAT);
    interval.tick().await;
    let result = loop {
        tokio::select! {
            result = &mut task => {
                break result.map_err(anyhow::Error::from).and_then(|result| result);
            }
            _ = interval.tick() => {
                let now = unix_now()?;
                let renewed = match metadata.jobs().renew_dependency_analysis_claim(&claim, now, LEASE_SECONDS).await {
                    Ok(renewed) => renewed,
                    Err(error) => { cancel_analysis(&cancellation, task).await; return Err(error.into()); }
                };
                if !renewed {
                    cancel_analysis(&cancellation, task).await;
                    tracing::debug!(repo_id = claim.incarnation.repository_id(), "dependency analysis claim superseded");
                    return Ok(PollOutcome::Worked);
                }
            }
            _ = &mut deadline => {
                cancel_analysis(&cancellation, task).await;
                break Err(anyhow::anyhow!("dependency analysis exceeded its total deadline"));
            }
            () = stop.set() => {
                cancel_analysis(&cancellation, task).await;
                return Ok(PollOutcome::Shutdown);
            }
        }
    };
    let completion = match result {
        Ok(output) => metadata
            .jobs()
            .complete_dependency_analysis_claim(&claim, output, unix_now()?)
            .await
            .map_err(anyhow::Error::from),
        Err(error) => Err(error),
    };
    let changed = match completion {
        Ok(completed) => completed == DependencyCompletion::Completed,
        Err(error) => {
            tracing::warn!(repo_id = claim.incarnation.repository_id(), %error, "dependency analysis failed");
            metadata
                .jobs()
                .fail_dependency_analysis_claim(&claim, PUBLIC_FAILURE, unix_now()?)
                .await?
        }
    };
    if changed {
        publish_change(state, &claim);
    }
    Ok(PollOutcome::Worked)
}

async fn cancel_analysis(
    cancellation: &ProcessCancellation,
    task: tokio::task::JoinHandle<anyhow::Result<AnalyzerOutput>>,
) {
    cancellation.cancel();
    if let Err(error) = task.await {
        tracing::warn!(%error, "dependency analysis task failed while cancelling");
    }
}

async fn analyze(
    state: &AppState,
    claim: &DependencyAnalysisClaim,
    cancellation: ProcessCancellation,
) -> anyhow::Result<AnalyzerOutput> {
    let snapshot = snapshot::materialize(state, claim, cancellation.clone()).await?;
    tokio::task::spawn_blocking(move || {
        let mut command = Command::new(std::env::current_exe()?);
        command
            .env_clear()
            .arg(ANALYZE_COMMAND)
            .arg(&snapshot.source);
        let output = run_cancellable(
            &mut command,
            None,
            ProcessLimits::new(ANALYZER_TIMEOUT).with_max_stdout_bytes(MAX_OUTPUT_BYTES),
            "analyzing repository dependencies",
            &cancellation,
        )?;
        if !output.status.success() {
            anyhow::bail!(
                "dependency analyzer failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(serde_json::from_slice(&output.stdout)?)
    })
    .await?
}

fn publish_change(state: &AppState, claim: &DependencyAnalysisClaim) {
    state.after_commit([AfterCommit::dependencies_changed(
        &claim.incarnation,
        claim.change_version,
    )]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    #[test]
    fn backfill_runs_at_startup_on_an_interval_and_while_candidates_remain() {
        let mut schedule = BackfillSchedule::default();
        let start = Instant::now();
        assert!(schedule.is_due(start));
        schedule.record(start, 0, 10);
        assert!(!schedule.is_due(start + Duration::from_secs(1)));
        assert!(schedule.is_due(start + BACKFILL_INTERVAL));
        schedule.record(start, 10, 10);
        assert!(schedule.is_due(start + Duration::from_secs(1)));
        schedule.record(start, 3, 10);
        assert!(!schedule.is_due(start + Duration::from_secs(1)));
    }

    #[tokio::test]
    async fn cancellation_waits_for_analysis_cleanup() {
        let cancellation = ProcessCancellation::new();
        let task_cancellation = cancellation.clone();
        let cleaned_up = Arc::new(AtomicBool::new(false));
        let task_cleaned_up = cleaned_up.clone();
        let task = tokio::spawn(async move {
            task_cancellation.cancelled().await;
            tokio::task::yield_now().await;
            task_cleaned_up.store(true, Ordering::SeqCst);
            anyhow::bail!("analysis canceled")
        });

        tokio::time::timeout(Duration::from_secs(5), cancel_analysis(&cancellation, task))
            .await
            .unwrap();
        assert!(cleaned_up.load(Ordering::SeqCst));
    }
}
