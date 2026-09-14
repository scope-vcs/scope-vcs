use crate::{
    error::ApiError,
    git::{
        GitContext,
        command::{run_git, truncated_git_stderr},
    },
};
use futures_util::{StreamExt as _, stream};
use scope_domain::repository::{
    RepositoryIncarnation,
    git::{GitHead, GitPackSpan, validate_git_pack_layout},
};
use scope_git::DEFAULT_GIT_BRANCH;
use scope_git_process::{ProcessLimits, run as run_process};
use std::{
    fs,
    io::{Read as _, Seek as _, SeekFrom},
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

pub(crate) async fn restore_git_pack_spans<C: GitContext>(
    context: &C,
    incarnation: &RepositoryIncarnation,
    head: &GitHead,
    pack_spans: &[GitPackSpan],
    repo_root: &Path,
) -> Result<(), ApiError> {
    let repository_id = incarnation.repository_id();
    let started_at = Instant::now();
    let total_pack_bytes = pack_spans
        .iter()
        .map(|span| span.segment.plaintext_bytes)
        .sum::<u64>();
    let result =
        restore_git_pack_spans_inner(context, incarnation, head, pack_spans, repo_root).await;
    tracing::info!(
        repository_id,
        operation = "restore_pack_layout",
        duration_ms = started_at.elapsed().as_millis(),
        requested_sequence = head.push_sequence,
        pack_span_count = pack_spans.len(),
        total_pack_bytes,
        success = result.is_ok(),
        "Git restore operation completed"
    );
    result
}

async fn restore_git_pack_spans_inner<C: GitContext>(
    context: &C,
    incarnation: &RepositoryIncarnation,
    head: &GitHead,
    pack_spans: &[GitPackSpan],
    repo_root: &Path,
) -> Result<(), ApiError> {
    let repository_id = incarnation.repository_id();
    validate_git_pack_layout(pack_spans)
        .map_err(|error| ApiError::internal_message(error.to_string()))?;
    let final_span = pack_spans
        .last()
        .ok_or_else(|| ApiError::internal_message("Git head has no physical pack spans"))?;
    if final_span.last_sequence != head.push_sequence || final_span.head_oid != head.head_oid {
        return Err(ApiError::internal_message(
            "Git pack layout frontier does not match the logical head",
        ));
    }
    let repo_root_for_cleanup = repo_root.to_path_buf();
    tokio::task::spawn_blocking(move || {
        if repo_root_for_cleanup.exists() {
            fs::remove_dir_all(repo_root_for_cleanup).map_err(ApiError::internal)?;
        }
        Ok::<_, ApiError>(())
    })
    .await
    .map_err(|error| {
        ApiError::internal_message(format!("Git restore cleanup task failed: {error}"))
    })??;
    run_timed_git_restore_phase_async(
        repository_id,
        "init",
        None,
        vec![
            "init".to_string(),
            "--bare".to_string(),
            repo_root.to_string_lossy().into_owned(),
        ],
        "initializing Git snapshot repo",
    )
    .await?;
    hydrate_git_pack_spans(context, repo_root, incarnation, pack_spans).await?;
    advance_head_and_verify(
        repository_id,
        repo_root,
        &head.head_oid,
        "restoring Git pack-layout head",
        "verifying restored Git pack layout",
    )
    .await?;
    run_timed_git_restore_phase_async(
        repository_id,
        "symbolic_ref",
        Some(repo_root.to_path_buf()),
        vec![
            "symbolic-ref".to_string(),
            "HEAD".to_string(),
            format!("refs/heads/{DEFAULT_GIT_BRANCH}"),
        ],
        "setting restored Git snapshot head",
    )
    .await?;
    Ok(())
}

/// Point the default branch at `head_oid`, then verify the object graph
/// reachable from it is complete.
pub(crate) async fn advance_head_and_verify(
    repository_id: &str,
    repo_root: &Path,
    head_oid: &str,
    update_ref_context: &'static str,
    fsck_context: &'static str,
) -> Result<(), ApiError> {
    run_timed_git_restore_phase_async(
        repository_id,
        "update_ref",
        Some(repo_root.to_path_buf()),
        vec![
            "update-ref".to_string(),
            format!("refs/heads/{DEFAULT_GIT_BRANCH}"),
            head_oid.to_string(),
        ],
        update_ref_context,
    )
    .await?;
    run_timed_git_restore_phase_async(
        repository_id,
        "fsck",
        Some(repo_root.to_path_buf()),
        vec![
            "fsck".to_string(),
            "--connectivity-only".to_string(),
            head_oid.to_string(),
        ],
        fsck_context,
    )
    .await
}

/// Fetch at most four packs ahead, then install them in layout order. The
/// repository engine serializes hydration for each incarnation.
pub(crate) async fn hydrate_git_pack_spans<C: GitContext>(
    context: &C,
    repo_root: &Path,
    incarnation: &RepositoryIncarnation,
    spans: &[GitPackSpan],
) -> Result<(), ApiError> {
    let mut packs = stream::iter(spans.iter().cloned().enumerate().map(
        |(index, span)| async move {
            let started = Instant::now();
            let pack = context
                .git_segment_store()
                .get_verified_pack(incarnation, &span.segment)
                .await;
            tracing::info!(
                phase = "verified", repository_id = incarnation.repository_id(),
                segment_id = span.segment.segment_id,
                source = ?pack.as_ref().ok().map(|pack| pack.timings().source),
                success = pack.is_ok(), duration_us = started.elapsed().as_micros(),
                bytes = span.segment.plaintext_bytes, "Git segment restore telemetry"
            );
            let pack =
                pack.map_err(|error| ApiError::infrastructure_unavailable(error.to_string()))?;
            Ok::<_, ApiError>((index, span, pack))
        },
    ))
    .buffered(4);
    while let Some(pack) = packs.next().await {
        let (index, span, pack) = pack?;
        let repository_id = incarnation.repository_id().to_string();
        let root = repo_root.to_path_buf();
        let count = spans.len();
        let timeout = context.runtime_budgets().git_command_timeout();
        tokio::task::spawn_blocking(move || {
            let started = Instant::now();
            let result = install_verified_git_pack(&root, pack.path(), timeout);
            tracing::info!(
                repository_id,
                operation = "index_pack",
                duration_ms = started.elapsed().as_millis(),
                span_index = index + 1,
                span_count = count,
                size_bytes = span.segment.plaintext_bytes,
                success = result.is_ok(),
                "Git restore operation completed"
            );
            result
        })
        .await
        .map_err(|error| {
            ApiError::internal_message(format!("Git index-pack task failed: {error}"))
        })??;
    }
    Ok(())
}

pub(super) fn install_verified_git_pack(
    repo_root: &Path,
    pack: &Path,
    timeout: Duration,
) -> Result<(), ApiError> {
    let index = pack.with_extension("idx");
    if !index.is_file() {
        let temporary_root = pack
            .parent()
            .and_then(Path::parent)
            .ok_or_else(|| ApiError::internal_message("verified Git pack has no cache directory"))?
            .join(".tmp");
        fs::create_dir_all(&temporary_root).map_err(ApiError::internal)?;
        let temporary = tempfile::Builder::new()
            .prefix("index-")
            .suffix(".idx")
            .tempfile_in(temporary_root)
            .map_err(ApiError::internal)?;
        let output = run_process(
            Command::new("git")
                .args(["index-pack", "--index-version=2", "-o"])
                .arg(temporary.path())
                .arg(pack),
            None,
            ProcessLimits::new(timeout),
            "indexing verified Git pack",
        )
        .map_err(|error| ApiError::infrastructure_unavailable(error.to_string()))?;
        if !output.status.success() {
            return Err(ApiError::infrastructure_unavailable(format!(
                "indexing verified Git pack: {}",
                truncated_git_stderr(&output.stderr).trim()
            )));
        }
        // index-pack replaces the output inode, so sync its completed file,
        // not the original empty NamedTempFile descriptor.
        fs::File::open(temporary.path())
            .and_then(|file| file.sync_all())
            .map_err(ApiError::internal)?;
        temporary
            .persist(&index)
            .map_err(|error| ApiError::internal(error.error))?;
        fs::File::open(index.parent().expect("index shares the pack directory"))
            .and_then(|directory| directory.sync_all())
            .map_err(ApiError::internal)?;
    }
    // Git names each pack after the SHA-1 trailer. The storage layer has already
    // authenticated the complete immutable pack against its durable metadata.
    let mut file = fs::File::open(pack).map_err(ApiError::internal)?;
    file.seek(SeekFrom::End(-20)).map_err(ApiError::internal)?;
    let mut trailer = [0_u8; 20];
    file.read_exact(&mut trailer).map_err(ApiError::internal)?;
    let basename = format!("pack-{}", hex::encode(trailer));
    let objects = repo_root.join("objects/pack");
    fs::create_dir_all(&objects).map_err(ApiError::internal)?;
    link_pack_file(pack, &objects.join(format!("{basename}.pack")))?;
    link_pack_file(&index, &objects.join(format!("{basename}.idx")))
}

fn link_pack_file(source: &Path, destination: &Path) -> Result<(), ApiError> {
    match fs::hard_link(source, destination) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::CrossesDevices => {
            fs::copy(source, destination)
                .map(|_| ())
                .map_err(ApiError::internal)
        }
        Err(error) => Err(ApiError::internal(error)),
    }
}

pub(crate) async fn run_timed_git_restore_phase_async(
    repository_id: &str,
    operation: &'static str,
    repo_root: Option<PathBuf>,
    args: Vec<String>,
    context: &'static str,
) -> Result<(), ApiError> {
    let repository_id = repository_id.to_string();
    tokio::task::spawn_blocking(move || {
        let args = args.iter().map(String::as_str).collect::<Vec<_>>();
        run_timed_git_restore_phase(
            &repository_id,
            operation,
            repo_root.as_deref(),
            &args,
            context,
        )
    })
    .await
    .map_err(|error| ApiError::internal_message(format!("Git restore task failed: {error}")))?
}

pub(crate) fn run_timed_git_restore_phase(
    repository_id: &str,
    operation: &'static str,
    repo_root: Option<&Path>,
    args: &[&str],
    context: &'static str,
) -> Result<(), ApiError> {
    let started_at = Instant::now();
    let result = run_git(repo_root, args, context);
    tracing::info!(
        repository_id,
        operation,
        duration_ms = started_at.elapsed().as_millis(),
        success = result.is_ok(),
        "Git restore operation completed"
    );
    result
}
