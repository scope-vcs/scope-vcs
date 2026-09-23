use super::request_ref_oid_is_commit;
use crate::{error::ApiError, git::command::run_git, state::AppState};
use scope_domain::content::SourceBlob;
use scope_object_store::source_blob_bytes;
use sha2::{Digest, Sha256};
use std::{fs, path::Path as FsPath};

/// Downloads a request snapshot bundle and fetches `request_ref` from it into `repo`. The bundle
/// lands in a temp file beside the repo and is removed whether or not the fetch succeeds.
pub(super) fn fetch_snapshot_into(
    state: &AppState,
    repo: &FsPath,
    request_ref: &str,
    snapshot: &SourceBlob,
    base_repo: Option<&FsPath>,
    action: &str,
) -> Result<(), ApiError> {
    let bundle_path = repo.with_extension(format!(
        "snapshot-{}.bundle.tmp",
        hex::encode(&Sha256::digest(format!("{request_ref}:{}", snapshot.sha256).as_bytes())[..8])
    ));
    let bytes = source_blob_bytes(state.object_store.as_ref(), snapshot)?;
    fs::write(&bundle_path, bytes).map_err(ApiError::internal)?;
    let result = fetch_bundle_into(repo, request_ref, &bundle_path, base_repo, action);
    let _ = fs::remove_file(&bundle_path);
    result
}

/// Fetches `request_ref` from a snapshot bundle file into `repo`, first supplying the request's
/// base from `base_repo` when `repo` lacks it.
pub(super) fn fetch_bundle_into(
    repo: &FsPath,
    request_ref: &str,
    bundle: &FsPath,
    base_repo: Option<&FsPath>,
    action: &str,
) -> Result<(), ApiError> {
    supply_bundle_base(repo, bundle, base_repo)?;
    let bundle = bundle.to_string_lossy();
    let refspec = format!("+{request_ref}:{request_ref}");
    run_git(
        Some(repo),
        &["fetch", "--no-tags", bundle.as_ref(), &refspec],
        action,
    )
}

/// A snapshot bundle leaves out the history of the request's base. Fetches any base commit
/// `repo` is missing, with its history, from `base_repo`.
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
        source.as_ref(),
    ];
    args.extend(missing.iter().map(String::as_str));
    run_git(Some(repo), &args, "fetching request snapshot base")
}

/// The commits a bundle requires but does not contain, read from its header.
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
