use crate::{
    error::ApiError,
    git::{
        command::git_command_output,
        import::{StagedGitPack, best_effort_delete_staged_git_segment, stage_git_pack},
    },
    persistence_ids::generate_prefixed_id,
    state::AppState,
};
use scope_domain::repository::{
    RepositoryIncarnation,
    git::{GitPoolPack, GitPoolPackOwner, GitPoolPackStorage},
};
use std::{
    fs,
    path::{Path as FsPath, PathBuf},
    process::Command,
    time::Instant,
};

const MAX_INLINE_POOL_PACK_BYTES: u64 = 1024 * 1024;

pub(crate) struct StagedPoolPack {
    pub(crate) pack: GitPoolPack,
    pub(crate) local: LocalPoolPack,
}

pub(crate) enum LocalPoolPack {
    Files(PoolPackFiles),
    Segment(StagedGitPack),
}

pub(crate) struct PoolPackFiles {
    _directory: tempfile::TempDir,
    pub(crate) checksum: String,
    pub(crate) pack: PathBuf,
    pub(crate) index: PathBuf,
}

pub(crate) struct PoolPackRevision {
    pub(crate) owner: GitPoolPackOwner,
    pub(crate) head_oid: String,
    pub(crate) prerequisites: Vec<String>,
}

pub(crate) struct RetainedLocalPack {
    path: PathBuf,
    _verified: Option<scope_storage::VerifiedGitPack>,
}

impl RetainedLocalPack {
    pub(crate) fn path(&self) -> &FsPath {
        &self.path
    }
}

impl StagedPoolPack {
    pub(crate) async fn discard(self, state: &AppState, repository_id: &str) {
        if let LocalPoolPack::Segment(staged) = self.local {
            best_effort_delete_staged_git_segment(state, repository_id, &staged.staged_segment)
                .await;
        }
    }

    pub(crate) async fn retain_local(
        &self,
        state: &AppState,
        incarnation: &RepositoryIncarnation,
    ) -> RetainedLocalPack {
        match &self.local {
            LocalPoolPack::Files(files) => RetainedLocalPack {
                path: files.pack.clone(),
                _verified: None,
            },
            LocalPoolPack::Segment(staged) => match state
                .git_segment_store
                .promote_verified_pack(incarnation, &staged.staged_segment)
                .await
            {
                Ok(verified) => RetainedLocalPack {
                    path: verified.path().to_path_buf(),
                    _verified: Some(verified),
                },
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        pack_id = self.pack.id,
                        "committed pool pack was not retained in the verified cache"
                    );
                    RetainedLocalPack {
                        path: staged.staged_segment.local_pack_path().to_path_buf(),
                        _verified: None,
                    }
                }
            },
        }
    }

    pub(crate) async fn finish(self, state: &AppState) {
        if let LocalPoolPack::Segment(staged) = &self.local
            && let Err(error) = state
                .git_segment_store
                .delete_local(&staged.staged_segment)
                .await
        {
            tracing::warn!(
                pack_id = self.pack.id,
                error = %error,
                "committed pool pack local staging cleanup failed"
            );
        }
    }
}

pub(crate) async fn stage_pool_pack(
    state: &AppState,
    repository_id: &str,
    staging_repo: &FsPath,
    revision: PoolPackRevision,
) -> Result<StagedPoolPack, ApiError> {
    let started = Instant::now();
    let revisions = std::iter::once(revision.head_oid.clone())
        .chain(revision.prerequisites.iter().map(|oid| format!("^{oid}")))
        .collect::<Vec<_>>();
    let files = {
        let staging_repo = staging_repo.to_path_buf();
        let revisions = revisions.clone();
        crate::git::blocking::run(move || write_pool_pack_files(&staging_repo, &revisions)).await?
    };
    let bytes = fs::metadata(&files.pack).map_err(ApiError::internal)?.len();
    let max_object_bytes = state
        .runtime_budgets
        .git_storage_limits()
        .max_object_bytes();
    if bytes > max_object_bytes as u64 {
        return Err(ApiError::payload_too_large(format!(
            "Git pack exceeds the {max_object_bytes} byte repository storage limit"
        )));
    }
    let (id, storage, local) = if bytes <= MAX_INLINE_POOL_PACK_BYTES {
        let id = generate_prefixed_id("git_pool_pack")?;
        let (files, sealed) = {
            let store = state.git_segment_store.clone();
            let repository_id = repository_id.to_string();
            let id = id.clone();
            crate::git::blocking::run(move || {
                let pack = fs::read(&files.pack).map_err(ApiError::internal)?;
                let sealed = store
                    .seal_inline_pack(&repository_id, &id, pack)
                    .map_err(|error| ApiError::internal_message(error.to_string()))?;
                Ok((files, sealed))
            })
            .await?
        };
        (
            id,
            GitPoolPackStorage::Inline { sealed },
            LocalPoolPack::Files(files),
        )
    } else {
        drop(files);
        let staged = stage_git_pack(state, repository_id, staging_repo, &revisions).await?;
        let segment = staged.staged_segment.segment.clone();
        (
            segment.segment_id.clone(),
            GitPoolPackStorage::Segment(segment),
            LocalPoolPack::Segment(staged),
        )
    };
    tracing::info!(
        repository_id,
        bytes,
        main = matches!(revision.owner, GitPoolPackOwner::Main { .. }),
        inline = matches!(storage, GitPoolPackStorage::Inline { .. }),
        duration_ms = started.elapsed().as_millis(),
        "pool pack staged"
    );
    Ok(StagedPoolPack {
        pack: GitPoolPack {
            id,
            owner: revision.owner,
            head_oid: revision.head_oid,
            prerequisites: revision.prerequisites,
            storage,
        },
        local,
    })
}

fn write_pool_pack_files(
    staging_repo: &FsPath,
    revisions: &[String],
) -> Result<PoolPackFiles, ApiError> {
    let parent = staging_repo
        .parent()
        .ok_or_else(|| ApiError::internal_message("staging repository has no parent"))?;
    let directory = tempfile::Builder::new()
        .prefix(".pool-pack-")
        .tempdir_in(parent)
        .map_err(ApiError::internal)?;
    let input = revisions
        .iter()
        .map(|revision| format!("{revision}\n"))
        .collect::<String>();
    let output = git_command_output(
        Command::new("git")
            .arg("-C")
            .arg(staging_repo)
            .args(["pack-objects", "--revs", "-q"])
            .arg(directory.path().join("pool")),
        Some(input.as_bytes()),
    )?;
    let checksum = String::from_utf8(output)
        .map_err(ApiError::internal)?
        .trim()
        .to_string();
    let base = directory.path().join(format!("pool-{checksum}"));
    Ok(PoolPackFiles {
        pack: base.with_extension("pack"),
        index: base.with_extension("idx"),
        checksum,
        _directory: directory,
    })
}
