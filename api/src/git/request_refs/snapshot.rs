use super::request_ref_oid_is_commit;
use crate::{error::ApiError, git::command::run_git, state::AppState};
use scope_domain::content::SourceBlob;
use scope_storage::{ObjectStore, ObjectStoreError, write_source_blob_to};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::Path as FsPath,
    sync::atomic::{AtomicU64, Ordering},
};

static SNAPSHOT_DOWNLOAD: AtomicU64 = AtomicU64::new(1);
pub(super) fn fetch_snapshot_into(
    state: &AppState,
    repo: &FsPath,
    request_ref: &str,
    snapshot: &SourceBlob,
    base_repo: Option<&FsPath>,
    action: &str,
) -> Result<Result<(), ObjectStoreError>, ApiError> {
    fetch_snapshot_into_ref(
        state,
        repo,
        BundleRef {
            source: request_ref,
            destination: request_ref,
        },
        snapshot,
        base_repo,
        action,
    )
}

pub(super) struct BundleRef<'a> {
    pub(super) source: &'a str,
    pub(super) destination: &'a str,
}

pub(super) fn fetch_snapshot_into_ref(
    state: &AppState,
    repo: &FsPath,
    bundle_ref: BundleRef<'_>,
    snapshot: &SourceBlob,
    base_repo: Option<&FsPath>,
    action: &str,
) -> Result<Result<(), ObjectStoreError>, ApiError> {
    let bundle_path = repo.with_extension(format!(
        "snapshot-{}-{}-{}.bundle.tmp",
        hex::encode(
            &Sha256::digest(format!("{}:{}", bundle_ref.destination, snapshot.sha256).as_bytes())
                [..8]
        ),
        std::process::id(),
        SNAPSHOT_DOWNLOAD.fetch_add(1, Ordering::Relaxed),
    ));
    let result =
        download_snapshot(state.object_store.as_ref(), snapshot, &bundle_path).and_then(|stored| {
            match stored {
                Ok(()) => {
                    fetch_bundle_into_ref(repo, bundle_ref, &bundle_path, base_repo, action).map(Ok)
                }
                Err(error) => Ok(Err(error)),
            }
        });
    let _ = fs::remove_file(&bundle_path);
    result
}

pub(super) fn download_snapshot(
    objects: &dyn ObjectStore,
    snapshot: &SourceBlob,
    path: &FsPath,
) -> Result<Result<(), ObjectStoreError>, ApiError> {
    crate::git::blocking::block_on(async {
        let mut file = tokio::fs::File::create(path)
            .await
            .map_err(ApiError::internal)?;
        Ok(write_source_blob_to(objects, snapshot, &mut file).await)
    })
}

pub(super) fn fetch_bundle_into(
    repo: &FsPath,
    request_ref: &str,
    bundle: &FsPath,
    base_repo: Option<&FsPath>,
    action: &str,
) -> Result<(), ApiError> {
    fetch_bundle_into_ref(
        repo,
        BundleRef {
            source: request_ref,
            destination: request_ref,
        },
        bundle,
        base_repo,
        action,
    )
}

fn fetch_bundle_into_ref(
    repo: &FsPath,
    bundle_ref: BundleRef<'_>,
    bundle: &FsPath,
    base_repo: Option<&FsPath>,
    action: &str,
) -> Result<(), ApiError> {
    supply_bundle_base(repo, bundle, base_repo)?;
    let bundle = bundle.to_string_lossy();
    let refspec = format!("+{}:{}", bundle_ref.source, bundle_ref.destination);
    run_git(
        Some(repo),
        &[
            "fetch",
            "--no-tags",
            "--no-write-fetch-head",
            bundle.as_ref(),
            &refspec,
        ],
        action,
    )
}

fn supply_bundle_base(
    repo: &FsPath,
    bundle: &FsPath,
    base_repo: Option<&FsPath>,
) -> Result<(), ApiError> {
    let mut missing = Vec::new();
    for oid in bundle_prerequisites(bundle)? {
        if !request_ref_oid_is_commit(repo, &oid)? {
            missing.push(oid);
        }
    }
    if missing.is_empty() {
        return Ok(());
    }
    let Some(base_repo) = base_repo else {
        return Err(ApiError::infrastructure_unavailable(
            "request snapshot base commit is unavailable",
        ));
    };
    let source = base_repo.to_string_lossy();
    let mut args = vec![
        "-c",
        "uploadpack.allowAnySHA1InWant=true",
        "fetch",
        "--no-tags",
        "--no-write-fetch-head",
        source.as_ref(),
    ];
    args.extend(missing.iter().map(String::as_str));
    run_git(Some(repo), &args, "fetching request snapshot base")
}

pub(super) fn bundle_prerequisites(bundle: &FsPath) -> Result<Vec<String>, ApiError> {
    use std::io::BufRead;
    let mut reader = std::io::BufReader::new(fs::File::open(bundle).map_err(ApiError::internal)?);
    let mut prerequisites = Vec::new();
    let mut line = Vec::new();
    loop {
        line.clear();
        if reader
            .read_until(b'\n', &mut line)
            .map_err(ApiError::internal)?
            == 0
            || line == b"\n"
        {
            return Ok(prerequisites);
        }
        if let Some(rest) = line.strip_prefix(b"-") {
            let oid = rest
                .split(|byte| byte.is_ascii_whitespace())
                .next()
                .unwrap_or_default();
            prerequisites.push(String::from_utf8(oid.to_vec()).map_err(ApiError::internal)?);
        }
    }
}
