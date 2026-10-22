use crate::{
    error::ApiError,
    git::{
        command::git_command_output,
        import::{StagedGitPack, best_effort_delete_staged_git_segment, stage_git_pack},
    },
    persistence_ids::generate_prefixed_id,
    state::AppState,
};
use scope_domain::repository::git::{GitPoolPack, GitPoolPackStorage};
use std::{
    fs,
    path::{Path as FsPath, PathBuf},
    process::Command,
    time::Instant,
};

const MAX_INLINE_POOL_PACK_BYTES: u64 = 1024 * 1024;

pub(crate) struct StagedPoolPack {
    pub(crate) pack: GitPoolPack,
    pub(super) local: LocalPoolPack,
}

pub(super) enum LocalPoolPack {
    Files(PoolPackFiles),
    Segment(StagedGitPack),
}

pub(super) struct PoolPackFiles {
    _directory: tempfile::TempDir,
    pub(super) checksum: String,
    pub(super) pack: PathBuf,
    pub(super) index: PathBuf,
}

pub(super) struct PoolPackRevision<'a> {
    pub(super) request_id: &'a str,
    pub(super) head_oid: &'a str,
    pub(super) prerequisites: Vec<String>,
}

impl StagedPoolPack {
    pub(crate) async fn discard(self, state: &AppState, repository_id: &str) {
        if let LocalPoolPack::Segment(staged) = self.local {
            best_effort_delete_staged_git_segment(state, repository_id, &staged.staged_segment)
                .await;
        }
    }
}

pub(super) async fn stage_pool_pack(
    state: &AppState,
    repository_id: &str,
    staging_repo: &FsPath,
    revision: PoolPackRevision<'_>,
) -> Result<StagedPoolPack, ApiError> {
    let started = Instant::now();
    let revisions = std::iter::once(revision.head_oid.to_string())
        .chain(revision.prerequisites.iter().map(|oid| format!("^{oid}")))
        .collect::<Vec<_>>();
    let files = {
        let staging_repo = staging_repo.to_path_buf();
        let revisions = revisions.clone();
        crate::git::blocking::run(move || write_pool_pack_files(&staging_repo, &revisions)).await?
    };
    let bytes = fs::metadata(&files.pack).map_err(ApiError::internal)?.len();
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
        inline = matches!(storage, GitPoolPackStorage::Inline { .. }),
        duration_ms = started.elapsed().as_millis(),
        "request pool pack staged"
    );
    Ok(StagedPoolPack {
        pack: GitPoolPack {
            id,
            request_id: revision.request_id.to_string(),
            head_oid: revision.head_oid.to_string(),
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
