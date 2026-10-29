use crate::{
    background_work::{BackgroundLoop, BackgroundWork, Cadence, Readiness},
    error::ApiError,
    git::request_refs::with_request_revision_store_repo,
    repo_events::RepoChangeReason,
    runtime_budgets::RuntimeBudgets,
    state::AppState,
    use_cases::{
        request_inspection_policy::MAX_REQUEST_INSPECTION_PATHS,
        request_revision_inspection::{
            parse_numstat_entry, read_nul_field_bounded, request_comparison_base_oid,
        },
    },
};
use scope_domain::{
    policy::ScopePath,
    repository::{RepoLifecycleState, RepositoryIncarnation, access::RepositoryAccessContext},
    requests::{
        DiffStat, Request, RequestComparison, RequestRevision, RevisionDiffStat,
        RevisionDiffStatBuilder,
    },
};
use scope_git_process::{ProcessLimits, StreamingProcessError, run_with_stdout, truncated_stderr};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::BufReader,
    path::Path,
    process::Command,
    time::Duration,
};

const BACKFILL_BATCH_SIZE: u64 = 16;
const BACKFILL_SWEEP_INTERVAL: Duration = Duration::from_secs(10 * 60);

pub(crate) async fn readable_request_diff_stats(
    state: &AppState,
    repository: &RepositoryAccessContext,
    request_ids: &[String],
) -> Result<BTreeMap<String, DiffStat>, ApiError> {
    let requests = state.metadata.requests();
    if repository.access.reads_full_view(&repository.views) {
        return Ok(requests.current_request_diff_totals(request_ids).await?);
    }
    let recorded = requests.current_request_diff_stats(request_ids).await?;
    if recorded.is_empty() {
        return Ok(BTreeMap::new());
    }
    let paths = recorded
        .values()
        .filter_map(RevisionDiffStat::files)
        .flatten()
        .map(|file| file.path.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let repositories = state.metadata.repositories();
    let read_policy = repositories.repository_policy_config(repository).await?;
    let history = repositories
        .repository_path_history(
            &repository.incarnation(),
            repository.record.content_version,
            &paths,
        )
        .await?;
    let reader = &repository.access.view;
    let policy =
        read_policy
            .policy
            .for_request_paths(&read_policy.repo_config, &history, reader, &paths);
    Ok(recorded
        .into_iter()
        .filter_map(|(request_id, stat)| {
            let readable =
                stat.readable_by(|path| policy.can_read(path, reader, &repository.views))?;
            Some((request_id, readable))
        })
        .collect())
}

pub(crate) async fn record_published_revision_diff_stat(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    request: &Request,
    revision: &RequestRevision,
) {
    match record_revision_diff_stat(state, incarnation, request, revision).await {
        Ok(true) => publish_recorded(state, incarnation, request).await,
        Ok(false) => {}
        Err(error) => tracing::warn!(
            request_id = %request.id,
            revision_id = %revision.id,
            error = %error.operator_diagnostic(),
            "recording request diff stat failed; the backfill will retry"
        ),
    }
}

async fn record_revision_diff_stat(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    request: &Request,
    revision: &RequestRevision,
) -> Result<bool, ApiError> {
    let view = request.view.clone();
    let stat = with_request_revision_store_repo(
        state,
        incarnation,
        request,
        revision,
        move |raw_repo, revision| {
            let comparison = RequestComparison::request(revision, view);
            let base_oid = request_comparison_base_oid(raw_repo, &comparison)?
                .ok_or_else(|| ApiError::internal_message("request comparison has no base"))?;
            revision_diff_stat(raw_repo, &base_oid, &comparison.identity().head_oid)
        },
    )
    .await?;
    Ok(state
        .metadata
        .requests()
        .record_request_revision_diff_stat(&revision.id, &stat)
        .await?)
}

fn revision_diff_stat(
    raw_repo: &Path,
    base_oid: &str,
    head_oid: &str,
) -> Result<RevisionDiffStat, ApiError> {
    let mut command = Command::new("git");
    command.arg("-C").arg(raw_repo).args([
        "--literal-pathspecs",
        "diff-tree",
        "-r",
        "--numstat",
        "-z",
        "--no-renames",
        base_oid,
        head_oid,
        "--",
    ]);
    let git_span = crate::git::command::prepare_git_subprocess(&mut command)?;
    let _entered = git_span.enter();
    let output = run_with_stdout(
        &mut command,
        None,
        ProcessLimits::new(RuntimeBudgets::default_git_command_timeout()),
        "counting request revision lines",
        move |stdout, _cancellation| {
            let mut reader = BufReader::new(stdout);
            let mut builder = RevisionDiffStatBuilder::new(MAX_REQUEST_INSPECTION_PATHS);
            while let Some(field) = read_nul_field_bounded(&mut reader)? {
                let (lines, path) = parse_numstat_entry(&field.bytes)?;
                let path = (!field.truncated)
                    .then(|| std::str::from_utf8(path).ok())
                    .flatten()
                    .and_then(|path| ScopePath::parse(format!("/{path}")).ok());
                builder.add(path, lines);
            }
            Ok(builder.finish())
        },
    )
    .map_err(|error| match error {
        StreamingProcessError::Process(error) => {
            ApiError::infrastructure_unavailable(error.to_string())
        }
        StreamingProcessError::Consumer(error) => error,
    })?;
    crate::git::command::record_git_exit(&git_span, output.status);
    if !output.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "counting request revision lines: {}",
            truncated_stderr(&output.stderr, scope_git_process::STDERR_DIAGNOSTIC_BYTES).trim()
        )));
    }
    Ok(output.value)
}

async fn publish_recorded(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    request: &Request,
) {
    state
        .publish_request_summary_refresh(incarnation, RepoChangeReason::RequestRevised)
        .await;
    state
        .publish_known_request_state_refresh(incarnation, request)
        .await;
}

#[derive(Clone, Default)]
pub(crate) struct DiffStatBackfillCursor {
    after_id: Option<String>,
}

impl BackgroundWork {
    pub(crate) fn start_request_diff_stat_backfill(&mut self) {
        self.spawn_with_progress(
            BackgroundLoop {
                name: "request diff stat backfill",
                cadence: Cadence::AfterPass {
                    delay: BACKFILL_SWEEP_INTERVAL,
                    wakeup: Some(|wakeups| &wakeups.request_diff_stats),
                },
                readiness: Readiness::SchemaRequired,
            },
            DiffStatBackfillCursor::default(),
            |state, mut cursor| async move {
                let result = backfill_request_diff_stats(&state, &mut cursor)
                    .await
                    .map(drop);
                (cursor, result)
            },
        );
    }
}

pub(crate) async fn backfill_request_diff_stats(
    state: &AppState,
    cursor: &mut DiffStatBackfillCursor,
) -> Result<usize, ApiError> {
    let requests = state
        .metadata
        .requests()
        .requests_missing_current_diff_stat(cursor.after_id.as_deref(), BACKFILL_BATCH_SIZE)
        .await?;
    cursor.after_id = if requests.len() as u64 == BACKFILL_BATCH_SIZE {
        state.wakeups.request_diff_stats.notify_one();
        requests.last().map(|request| request.id.clone())
    } else {
        None
    };
    let mut recorded = 0;
    for request in requests {
        match record_current_revision_diff_stat(state, &request).await {
            Ok(true) => recorded += 1,
            Ok(false) => {}
            Err(error) => tracing::warn!(
                request_id = %request.id,
                error = %error.operator_diagnostic(),
                "backfilling request diff stat failed"
            ),
        }
    }
    Ok(recorded)
}

async fn record_current_revision_diff_stat(
    state: &AppState,
    request: &Request,
) -> Result<bool, ApiError> {
    let Some(repo) = state
        .metadata
        .repositories()
        .repository_record(&request.repo_id)
        .await?
        .filter(|repo| repo.lifecycle_state == RepoLifecycleState::Ready)
    else {
        return Ok(false);
    };
    let Some(revision) = state
        .metadata
        .requests()
        .latest_request_revision(&request.id)
        .await?
    else {
        return Ok(false);
    };
    let incarnation = repo.incarnation();
    let recorded = record_revision_diff_stat(state, &incarnation, request, &revision).await?;
    if recorded {
        publish_recorded(state, &incarnation, request).await;
    }
    Ok(recorded)
}
