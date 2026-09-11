mod snapshot;

use crate::{
    health::WorkerHealth,
    settings::{WorkerRole, WorkerSettings},
};
use scope_api_contract::{RepoChangeEvent, RepoChangeKind, RepoChangeNotification};
use scope_domain::dependency_analysis::{AnalyzerOutput, DEPENDENCY_ANALYZER_VERSION};
use scope_git_process::{ProcessCancellation, ProcessLimits, run as run_process, run_cancellable};
use scope_git_storage::GitSegmentStore;
use scope_object_store::ObjectStore;
use scope_postgres::db::{DependencyAnalysisClaim, DependencyCompletion, MetadataStore};
use std::{path::PathBuf, process::Command, sync::Arc, time::Duration};

const LEASE_SECONDS: u64 = 60;
const HEARTBEAT: Duration = Duration::from_secs(5);
const ANALYZER_TIMEOUT: Duration = Duration::from_secs(120);
const ANALYSIS_TIMEOUT: Duration = Duration::from_secs(300);
const MAX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
#[derive(Clone, Copy, PartialEq, Eq)]
enum PollOutcome {
    Idle,
    Worked,
    Shutdown,
}

const PUBLIC_FAILURE: &str = "Dependency check could not finish. It will retry automatically.";

fn analyzer_command() -> anyhow::Result<Command> {
    let path = std::env::var_os("SCOPE_DEPENDENCY_ANALYZER_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("dependency-analyzer/analyze.mjs"))
        .canonicalize()
        .map_err(|error| anyhow::anyhow!("dependency analyzer is not installed: {error}"))?;
    let mut command = Command::new("node");
    command.env_clear();
    if let Some(path) = std::env::var_os("PATH") {
        command.env("PATH", path);
    }
    command.arg("--max-old-space-size=512").arg(path);
    Ok(command)
}

pub(crate) fn require_analyzer_runtime() -> anyhow::Result<()> {
    let output = run_process(
        analyzer_command()?.arg("--version"),
        None,
        ProcessLimits::new(Duration::from_secs(10)).with_max_stdout_bytes(256),
        "checking dependency analyzer runtime",
    )?;
    if !output.status.success()
        || String::from_utf8_lossy(&output.stdout).trim() != DEPENDENCY_ANALYZER_VERSION
    {
        anyhow::bail!("dependency analyzer runtime must be {DEPENDENCY_ANALYZER_VERSION}");
    }
    Ok(())
}

pub(crate) async fn run(
    metadata: MetadataStore,
    objects: Arc<dyn ObjectStore>,
    segments: Arc<GitSegmentStore>,
    settings: WorkerSettings,
    health: WorkerHealth,
) -> anyhow::Result<()> {
    loop {
        if !crate::schema_ready_or_wait(&metadata, &health).await {
            return Ok(());
        }
        let result = process_next(
            &metadata,
            objects.clone(),
            segments.clone(),
            &settings,
            &health,
        )
        .await;
        match result {
            Ok(PollOutcome::Shutdown) => return Ok(()),
            Ok(outcome) => {
                health.mark_poll_succeeded(WorkerRole::Dependencies, crate::unix_now()?);
                if outcome == PollOutcome::Worked {
                    continue;
                }
            }
            Err(error) => {
                tracing::error!(%error, "dependency analysis scheduling failed; retrying");
            }
        }
        if crate::wait_or_shutdown(settings.poll_interval).await {
            return Ok(());
        }
    }
}

async fn process_next(
    metadata: &MetadataStore,
    objects: Arc<dyn ObjectStore>,
    segments: Arc<GitSegmentStore>,
    settings: &WorkerSettings,
    health: &WorkerHealth,
) -> anyhow::Result<PollOutcome> {
    let now = crate::unix_now()?;
    metadata
        .jobs()
        .enqueue_dependency_analysis_backfill(DEPENDENCY_ANALYZER_VERSION, now, settings.batch_size)
        .await?;
    let Some(claim) = metadata
        .jobs()
        .claim_dependency_analysis(
            &settings.worker_id,
            DEPENDENCY_ANALYZER_VERSION,
            now,
            LEASE_SECONDS,
            &crate::generate_persistence_id,
        )
        .await?
    else {
        return Ok(PollOutcome::Idle);
    };
    if claim.reusable_analysis.is_some() {
        let completed = metadata
            .jobs()
            .complete_reused_dependency_analysis_claim(&claim, crate::unix_now()?)
            .await?;
        if completed == DependencyCompletion::Completed {
            publish_change(metadata, &settings.worker_id, &claim).await;
        }
        return Ok(PollOutcome::Worked);
    }
    let task_claim = claim.clone();
    let data_dir = settings.data_dir.clone();
    let cancellation = ProcessCancellation::new();
    let task_cancellation = cancellation.clone();
    let mut task = tokio::spawn(async move {
        analyze(&task_claim, objects, segments, data_dir, task_cancellation).await
    });
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
                let now = crate::unix_now()?;
                let renewed = match metadata.jobs().renew_dependency_analysis_claim(&claim, now, LEASE_SECONDS).await {
                    Ok(renewed) => renewed,
                    Err(error) => { cancel_analysis(&cancellation, task).await; return Err(error.into()); }
                };
                if !renewed {
                    cancel_analysis(&cancellation, task).await;
                    tracing::debug!(repo_id = claim.incarnation.repository_id(), "dependency analysis claim superseded");
                    return Ok(PollOutcome::Worked);
                }
                health.mark_poll_succeeded(WorkerRole::Dependencies, now);
            }
            _ = &mut deadline => {
                cancel_analysis(&cancellation, task).await;
                break Err(anyhow::anyhow!("dependency analysis exceeded its total deadline"));
            }
            _ = crate::shutdown_signal() => {
                cancel_analysis(&cancellation, task).await;
                return Ok(PollOutcome::Shutdown);
            }
        }
    };
    let completion = match result {
        Ok(output) => metadata
            .jobs()
            .complete_dependency_analysis_claim(&claim, output, crate::unix_now()?)
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
                .fail_dependency_analysis_claim(&claim, PUBLIC_FAILURE, crate::unix_now()?)
                .await?
        }
    };
    if changed {
        publish_change(metadata, &settings.worker_id, &claim).await;
    }
    Ok(PollOutcome::Worked)
}

async fn cancel_analysis(
    cancellation: &ProcessCancellation,
    task: tokio::task::JoinHandle<anyhow::Result<AnalyzerOutput>>,
) {
    cancellation.cancel();
    let _ = task.await;
}

async fn analyze(
    claim: &DependencyAnalysisClaim,
    objects: Arc<dyn ObjectStore>,
    segments: Arc<GitSegmentStore>,
    data_dir: PathBuf,
    cancellation: ProcessCancellation,
) -> anyhow::Result<AnalyzerOutput> {
    let snapshot =
        snapshot::materialize(claim, objects, segments, &data_dir, cancellation.clone()).await?;
    tokio::task::spawn_blocking(move || {
        let output = run_cancellable(
            analyzer_command()?.arg(&snapshot.source),
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
        let analysis: AnalyzerOutput = serde_json::from_slice(&output.stdout)?;
        if analysis.analyzer_version != DEPENDENCY_ANALYZER_VERSION {
            anyhow::bail!("dependency analyzer version changed during analysis");
        }
        Ok(analysis)
    })
    .await?
}

async fn publish_change(
    metadata: &MetadataStore,
    worker_id: &str,
    claim: &DependencyAnalysisClaim,
) {
    let notification = RepoChangeNotification {
        event: RepoChangeEvent {
            repo_id: claim.incarnation.repository_id().to_string(),
            incarnation_id: claim.incarnation.incarnation_id().to_string(),
            version: claim.repo_version,
            kind: RepoChangeKind::DependenciesChanged,
        },
        origin_id: worker_id.to_string(),
    };
    let result = async {
        let payload = serde_json::to_string(&notification)?;
        metadata.repositories().notify_repo_change(&payload).await?;
        Ok::<_, anyhow::Error>(())
    }
    .await;
    if let Err(error) = result {
        tracing::warn!(repo_id = claim.incarnation.repository_id(), %error, "failed to publish dependency report change");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

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
