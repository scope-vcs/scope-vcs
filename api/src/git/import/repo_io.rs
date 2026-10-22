use super::segment_upload::{
    GitSegmentUploadHeartbeat, RemoteCleanup, begin_git_segment_upload,
    best_effort_delete_git_segment_identity, best_effort_delete_staged_git_segment,
};
use crate::{
    error::ApiError,
    git::command::{
        git_ref_listing, git_stdout_text, prepare_git_subprocess, record_git_exit, run_git,
        truncated_git_stderr,
    },
    state::AppState,
};
use scope_domain::content::SourceBlob;
use scope_domain::repository::git::GitHead;
use scope_git::DEFAULT_GIT_BRANCH;
use scope_git::{StoredGitPush, prepare_git_push};
use scope_git_process::{ProcessLimits, StreamingProcessError, run_with_stdout};
use scope_storage::{ContentObjectKind, content_object_for_bytes};
use scope_storage::{GitStorageError, StagedGitSegment};
use std::{path::Path as FsPath, process::Command, time::Instant};

pub(super) struct PushedCommit {
    pub(super) message: String,
    pub(super) committed_at_unix: i64,
}

pub(super) fn pushed_commit(
    staging_repo: &FsPath,
    head_oid: &str,
) -> Result<PushedCommit, ApiError> {
    let output = git_stdout_text(
        staging_repo,
        &["log", "-1", "--format=%ct%n%B", head_oid],
        "reading pushed commit",
    )?;
    let (committed_at, message) = output.split_once('\n').unwrap_or((output.as_str(), ""));
    let committed_at_unix = committed_at
        .trim()
        .parse()
        .map_err(|_| ApiError::internal_message("invalid pushed Git commit time"))?;
    let message = message.trim_end_matches(&['\r', '\n'][..]);
    let message = if message.trim().is_empty() {
        format!("Push to {DEFAULT_GIT_BRANCH}")
    } else {
        message.to_string()
    };
    Ok(PushedCommit {
        message,
        committed_at_unix,
    })
}

pub(crate) fn git_refs(staging_repo: &FsPath) -> Result<Vec<(String, String)>, ApiError> {
    let main_ref = format!("refs/heads/{DEFAULT_GIT_BRANCH}");
    git_ref_listing(
        staging_repo,
        &[&main_ref, "refs/tags"],
        "reading pushed refs",
    )
}

pub(super) fn describe_refs(refs: &[(String, String)]) -> String {
    if refs.is_empty() {
        return "none".to_string();
    }

    refs.iter()
        .map(|(name, oid)| format!("{name}@{}", oid.get(..12).unwrap_or(oid)))
        .collect::<Vec<_>>()
        .join(", ")
}

pub(crate) struct StagedGitPush {
    pub(crate) stored: StoredGitPush,
    pub(crate) staged_segment: StagedGitSegment,
    pub(crate) upload_heartbeat: GitSegmentUploadHeartbeat,
}

pub(crate) async fn git_push_from_repo(
    state: &AppState,
    repository_id: &str,
    repo: &FsPath,
    previous: Option<&GitHead>,
) -> Result<StagedGitPush, ApiError> {
    let _ingest_permit = state.runtime_budgets.try_git_segment_ingest()?;
    let storage_limits = state.runtime_budgets.git_storage_limits();
    let path = repo.to_path_buf();
    let (head_oid, ingest_permit) = crate::git::blocking::run(move || {
        let refname = format!("refs/heads/{DEFAULT_GIT_BRANCH}");
        let head = git_stdout_text(&path, &["rev-parse", &refname], "reading pushed Git head")?;
        Ok((head.trim().to_string(), _ingest_permit))
    })
    .await?;
    let mut revisions = format!("{head_oid}\n");
    if let Some(previous) = previous {
        revisions.push('^');
        revisions.push_str(&previous.head_oid);
        revisions.push('\n');
    }
    let mut command = Command::new("git");
    command
        .current_dir(repo)
        .args(["pack-objects", "--revs", "--stdout"]);
    let git_span = prepare_git_subprocess(&mut command)?;
    let (reservation, upload_heartbeat) = begin_git_segment_upload(state, repository_id).await?;
    let segment_id = reservation.segment_id.clone();
    let object_key = reservation.object_key.clone();

    let pack_started = Instant::now();
    let segment_store = state.git_segment_store.clone();
    let repository_id_for_ingest = repository_id.to_string();
    let timeout = state.runtime_budgets.git_command_timeout();
    let parent_span = tracing::Span::current();
    let output = tokio::task::spawn_blocking(move || {
        let _entered = parent_span.enter();
        let _ingest_permit = ingest_permit;
        let runtime = tokio::runtime::Handle::current();
        let _entered = git_span.enter();
        let output = run_with_stdout(
            &mut command,
            Some(revisions.into_bytes()),
            ProcessLimits::new(timeout),
            "creating incremental Git pack",
            move |stdout, cancellation| {
                runtime.block_on(segment_store.ingest_reserved_blocking_reader(
                    &repository_id_for_ingest,
                    reservation,
                    stdout,
                    storage_limits.max_object_bytes() as u64,
                    Some(cancellation),
                ))
            },
        );
        if let Ok(output) = &output {
            record_git_exit(&git_span, output.status);
        }
        output
    })
    .await;
    let output = match output {
        Ok(output) => output,
        Err(error) => {
            best_effort_delete_git_segment_identity(
                state,
                repository_id,
                &segment_id,
                &object_key,
                RemoteCleanup::Attempt,
            )
            .await;
            return Err(ApiError::internal(error));
        }
    };
    let pack_elapsed = pack_started.elapsed();
    let output = match output {
        Ok(output) => output,
        Err(error) => {
            let remote_cleanup = if matches!(
                &error,
                StreamingProcessError::Process(error) if error.is_timeout()
            ) {
                RemoteCleanup::Deferred
            } else {
                RemoteCleanup::Attempt
            };
            best_effort_delete_git_segment_identity(
                state,
                repository_id,
                &segment_id,
                &object_key,
                remote_cleanup,
            )
            .await;
            return Err(match error {
                StreamingProcessError::Process(error) => {
                    ApiError::infrastructure_unavailable(error.to_string())
                }
                StreamingProcessError::Consumer(GitStorageError::PlaintextLimitExceeded {
                    ..
                }) => ApiError::payload_too_large(format!(
                    "Git pack exceeds the {} byte repository storage limit",
                    storage_limits.max_object_bytes()
                )),
                StreamingProcessError::Consumer(error) => {
                    ApiError::infrastructure_unavailable(error.to_string())
                }
            });
        }
    };
    let staged_segment = output.value;
    if !output.status.success() {
        tracing::info!(
            incremental = previous.is_some(),
            pack_us = pack_elapsed.as_micros(),
            pack_bytes = staged_segment.segment.plaintext_bytes,
            success = false,
            "Git push pack timing"
        );
        best_effort_delete_staged_git_segment(state, repository_id, &staged_segment).await;
        return Err(ApiError::infrastructure_unavailable(format!(
            "creating incremental Git pack: {}",
            truncated_git_stderr(&output.stderr).trim()
        )));
    }
    if let Err(error) = state
        .metadata
        .repositories()
        .mark_git_segment_upload_ready(
            &staged_segment.segment,
            staged_segment.encrypted_bytes,
            crate::persistence::unix_now()?,
        )
        .await
    {
        best_effort_delete_staged_git_segment(state, repository_id, &staged_segment).await;
        return Err(error.into());
    }

    let pack_bytes = staged_segment.segment.plaintext_bytes;
    let store_started = Instant::now();
    let stored = match prepare_git_push(
        staged_segment.segment.clone(),
        head_oid,
        previous,
        storage_limits,
    ) {
        Ok(prepared) => prepared,
        Err(error) => {
            best_effort_delete_staged_git_segment(state, repository_id, &staged_segment).await;
            return Err(error.into());
        }
    };
    let timings = &staged_segment.timings;
    tracing::info!(
        phase = "complete",
        repository_id,
        segment_id = staged_segment.segment.segment_id,
        success = true,
        duration_us = timings.total.as_micros(),
        local_write_us = timings.local_write.as_micros(),
        remote_upload_us = timings.remote_upload.as_micros(),
        uploaded_parts = timings.uploaded_parts,
        bytes = timings.plaintext_bytes,
        blocked_us = timings.fanout_blocked.as_micros(),
        configured_buffer_capacity_bytes =
            timings.chunk_bytes.saturating_mul(timings.channel_capacity),
        "Git segment ingest telemetry"
    );
    tracing::info!(
        incremental = previous.is_some(),
        pack_us = pack_elapsed.as_micros(),
        store_us = store_started.elapsed().as_micros(),
        pack_bytes,
        success = true,
        "Git push pack timing"
    );
    Ok(StagedGitPush {
        stored,
        staged_segment,
        upload_heartbeat,
    })
}

pub(crate) fn git_snapshot_from_ref(
    repo: &FsPath,
    refname: &str,
    base_oid: Option<&str>,
) -> Result<(SourceBlob, Vec<u8>), ApiError> {
    let head_oid = git_stdout_text(repo, &["rev-parse", refname], "reading Git snapshot head")?;
    let head_oid = head_oid.trim();
    let bundle_path = repo.join(format!("scope-snapshot-{}.bundle", random_bundle_id()?));
    let bundle = bundle_path.to_string_lossy().to_string();
    let exclude_base = base_oid
        .filter(|base_oid| *base_oid != head_oid)
        .map(|base_oid| format!("^{base_oid}"));
    let mut args = vec!["bundle", "create", bundle.as_str()];
    args.extend(exclude_base.as_deref());
    args.push(refname);
    run_git(Some(repo), &args, "creating Git snapshot bundle")?;
    let bytes = std::fs::read(&bundle_path).map_err(ApiError::internal)?;
    let _ = std::fs::remove_file(&bundle_path);
    let mut snapshot = content_object_for_bytes(ContentObjectKind::GitBundle, &bytes);
    snapshot.git_oid = head_oid.to_string();
    Ok((snapshot, bytes))
}

fn random_bundle_id() -> Result<String, ApiError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| {
        ApiError::internal_message(format!("Git snapshot bundle id generation failed: {error}"))
    })?;
    Ok(format!("{}-{}", std::process::id(), hex::encode(bytes)))
}
