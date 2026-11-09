use crate::error::ApiError;
use scope_domain::repository::git::{
    GitHead, GitPoolPack, GitPoolPackStorage, GitSegmentUpload, GitSegmentUploadState,
};
use std::{
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

pub(super) struct SeedPoolPack {
    pack: GitPoolPack,
    upload: GitSegmentUpload,
}

impl SeedPoolPack {
    pub(super) fn add_to(self, catalog: &mut scope_postgres::db::CatalogFixture) {
        catalog
            .git_pool_packs
            .entry(self.upload.repository_id.clone())
            .or_default()
            .push(self.pack);
        catalog.git_segment_uploads.push(self.upload);
    }
}

pub(super) fn store_seed_git_pack(
    git_segment_store: &scope_storage::GitSegmentStore,
    repository_id: &str,
    repo_path: &Path,
) -> Result<(GitHead, SeedPoolPack), ApiError> {
    store_seed_head_pack(
        git_segment_store,
        repository_id,
        repo_path,
        super::seed_git_head(repo_path)?,
    )
}

pub(super) fn store_seed_request_pack(
    git_segment_store: &scope_storage::GitSegmentStore,
    repository_id: &str,
    repo_path: &Path,
    request_id: &str,
    head_oid: &str,
    view: &scope_domain::views::ViewId,
) -> Result<SeedPoolPack, ApiError> {
    let (_, mut stored) = store_seed_head_pack(
        git_segment_store,
        repository_id,
        repo_path,
        head_oid.to_string(),
    )?;
    stored.pack.owner = scope_domain::repository::git::GitPoolPackOwner::Request {
        request_id: request_id.to_string(),
        scope: scope_domain::repository::git::RequestPackScope::for_view(
            view,
            &scope_domain::views::Views::builtin(),
        ),
    };
    Ok(stored)
}

fn store_seed_head_pack(
    git_segment_store: &scope_storage::GitSegmentStore,
    repository_id: &str,
    repo_path: &Path,
    head_oid: String,
) -> Result<(GitHead, SeedPoolPack), ApiError> {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .args(["pack-objects", "--revs", "--stdout"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(ApiError::internal)?;
    child
        .stdin
        .take()
        .ok_or_else(|| ApiError::internal_message("seed pack stdin unavailable"))?
        .write_all(format!("{head_oid}\n").as_bytes())
        .map_err(ApiError::internal)?;
    let output = child.wait_with_output().map_err(ApiError::internal)?;
    if !output.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "creating seeded Git pack: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    let store = git_segment_store.clone();
    let repository_id = repository_id.to_string();
    let ingest_repository_id = repository_id.clone();
    let staged = std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(scope_storage::GitStorageError::Local)?
            .block_on(store.ingest_blocking_reader(
                &ingest_repository_id,
                std::io::Cursor::new(output.stdout),
                scope_git::GitStorageLimits::default().max_object_bytes() as u64,
            ))
    })
    .join()
    .map_err(|_| ApiError::internal_message("seed Git segment upload thread panicked"))?
    .map_err(|error| ApiError::infrastructure_unavailable(error.to_string()))?;
    let prepared =
        scope_git::prepare_git_push(head_oid, None, scope_git::GitStorageLimits::default())?;
    let upload = GitSegmentUpload {
        segment_id: staged.segment.segment_id.clone(),
        repository_id,
        object_key: staged.object_key,
        state: GitSegmentUploadState::Published,
        sha256: Some(staged.segment.sha256.clone()),
        plaintext_bytes: Some(staged.segment.plaintext_bytes),
        encrypted_bytes: Some(staged.encrypted_bytes),
        encoding_version: staged.segment.encoding_version,
        created_at_unix: 1_800_000_000,
        updated_at_unix: 1_800_000_000,
    };
    let pack = GitPoolPack {
        id: staged.segment.segment_id.clone(),
        owner: prepared.owner,
        head_oid: prepared.head.head_oid.clone(),
        prerequisites: prepared.prerequisites,
        storage: GitPoolPackStorage::Segment(staged.segment),
    };
    Ok((prepared.head, SeedPoolPack { pack, upload }))
}
