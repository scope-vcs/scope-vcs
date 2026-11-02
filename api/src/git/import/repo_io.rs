use super::segment_upload::{
    GitSegmentUploadHeartbeat, begin_git_segment_upload, best_effort_delete_git_segment_identity,
    best_effort_delete_staged_git_segment,
};
use crate::{
    error::ApiError,
    git::{
        command::{git_ref_listing, git_stdout_text, run_git},
        pool_pack::{PoolPackRevision, StagedPoolPack, stage_pool_pack},
    },
    state::AppState,
};
use scope_domain::content::SourceBlob;
use scope_domain::repository::git::{GitHead, GitPoolPackStorage};
use scope_git::DEFAULT_GIT_BRANCH;
use scope_git::{PreparedGitPush, prepare_git_push};
use scope_storage::StagedGitSegment;
use scope_storage::{ContentObjectKind, content_object_for_bytes};
use std::{path::Path as FsPath, time::Instant};

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
    pub(crate) head: GitHead,
    pub(crate) pack: StagedPoolPack,
}

pub(crate) struct StagedGitPack {
    pub(crate) staged_segment: StagedGitSegment,
    pub(crate) _upload_heartbeat: GitSegmentUploadHeartbeat,
}

pub(crate) async fn stage_git_pack_file(
    state: &AppState,
    repository_id: &str,
    path: &FsPath,
) -> Result<StagedGitPack, ApiError> {
    let _ingest_permit = state.runtime_budgets.try_git_segment_ingest()?;
    let source = std::fs::File::open(path).map_err(ApiError::internal)?;
    let (reservation, upload_heartbeat) = begin_git_segment_upload(state, repository_id).await?;
    let segment_id = reservation.segment_id.clone();
    let object_key = reservation.object_key.clone();
    let staged_segment = match state
        .git_segment_store
        .ingest_reserved_blocking_reader(
            repository_id,
            reservation,
            source,
            state
                .runtime_budgets
                .git_storage_limits()
                .max_object_bytes() as u64,
            None,
        )
        .await
    {
        Ok(staged) => staged,
        Err(error) => {
            best_effort_delete_git_segment_identity(state, repository_id, &segment_id, &object_key)
                .await;
            return Err(ApiError::infrastructure_unavailable(error.to_string()));
        }
    };
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
    Ok(StagedGitPack {
        staged_segment,
        _upload_heartbeat: upload_heartbeat,
    })
}

pub(crate) async fn git_push_from_repo(
    state: &AppState,
    repository_id: &str,
    repo: &FsPath,
    previous: Option<&GitHead>,
) -> Result<StagedGitPush, ApiError> {
    let path = repo.to_path_buf();
    let head_oid = crate::git::blocking::run(move || {
        let refname = format!("refs/heads/{DEFAULT_GIT_BRANCH}");
        let head = git_stdout_text(&path, &["rev-parse", &refname], "reading pushed Git head")?;
        Ok(head.trim().to_string())
    })
    .await?;
    let PreparedGitPush {
        head,
        owner,
        prerequisites,
    } = prepare_git_push(
        head_oid.clone(),
        previous,
        state.runtime_budgets.git_storage_limits(),
    )?;
    let pack_started = Instant::now();
    let pack = stage_pool_pack(
        state,
        repository_id,
        repo,
        PoolPackRevision {
            owner,
            head_oid,
            prerequisites,
        },
    )
    .await?;
    tracing::info!(
        incremental = previous.is_some(),
        pack_us = pack_started.elapsed().as_micros(),
        inline = matches!(pack.pack.storage, GitPoolPackStorage::Inline { .. }),
        pack_bytes = pack.pack.storage.stored_bytes(),
        success = true,
        "Git push pack timing"
    );
    Ok(StagedGitPush { head, pack })
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
