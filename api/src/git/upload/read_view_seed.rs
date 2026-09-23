use crate::{
    error::ApiError,
    git::{
        command::{git_ref_listing, run_git},
        repository_engine::RepositoryEngine,
    },
};
use scope_domain::requests::{Request, canonical_request_ref};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::SystemTime,
};

/// Copies request refs whose metadata head already exists in an earlier read view of the same
/// repository incarnation, so only requests that actually changed need their snapshot bundle from
/// the object store. A ref is fetched by name from a view that holds the exact head, and only
/// counts as attached once the target repo shows that head, so an earlier view can never supply
/// stale or foreign objects. Returns the names of the requests attached this way.
pub(super) fn seed_request_refs_from_read_views(
    engine: &Arc<RepositoryEngine>,
    read_view_prefix: &str,
    requests: &[Request],
    target_repo: &Path,
) -> Result<BTreeSet<String>, ApiError> {
    let mut remaining: BTreeMap<String, &Request> = requests
        .iter()
        .filter(|request| request.git_snapshot.is_some())
        .map(|request| (canonical_request_ref(&request.name), request))
        .collect();
    let mut seeded = BTreeSet::new();
    if remaining.is_empty() {
        return Ok(seeded);
    }
    for candidate in earlier_read_views(engine.cache_root(), read_view_prefix)? {
        if remaining.is_empty() {
            break;
        }
        // The lease keeps the reaper from evicting the view mid-fetch.
        let Ok(view) = engine.lease_derived(candidate) else {
            continue;
        };
        let Ok(refs) = git_ref_listing(
            view.as_ref(),
            &["refs/heads"],
            "listing earlier Git read view refs",
        ) else {
            continue;
        };
        let refspecs: Vec<String> = refs
            .iter()
            .filter(|(refname, oid)| {
                remaining
                    .get(refname)
                    .is_some_and(|request| request.head_oid == *oid)
            })
            .map(|(refname, _)| format!("+{refname}:{refname}"))
            .collect();
        if refspecs.is_empty() {
            continue;
        }
        let source = view.as_ref().to_string_lossy().to_string();
        let mut args = vec!["fetch", "--no-tags", source.as_str()];
        args.extend(refspecs.iter().map(String::as_str));
        if let Err(error) = run_git(
            Some(target_repo),
            &args,
            "seeding request refs from an earlier Git read view",
        ) {
            tracing::warn!(
                source = %view.as_ref().display(),
                error = %error.operator_diagnostic(),
                "earlier Git read view could not seed request refs"
            );
            continue;
        }
        for (refname, oid) in git_ref_listing(
            target_repo,
            &["refs/heads"],
            "verifying seeded request refs",
        )? {
            if remaining
                .get(&refname)
                .is_some_and(|request| request.head_oid == oid)
                && let Some(request) = remaining.remove(&refname)
            {
                seeded.insert(request.name.clone());
            }
        }
    }
    Ok(seeded)
}

/// Materialized read views for the incarnation, newest first. The view under construction is a
/// `.tmp` directory and is never listed.
fn earlier_read_views(cache_root: &Path, read_view_prefix: &str) -> Result<Vec<PathBuf>, ApiError> {
    let directory_prefix = format!("read-view-{read_view_prefix}-");
    let mut views = Vec::new();
    for entry in fs::read_dir(cache_root).map_err(ApiError::internal)? {
        let entry = entry.map_err(ApiError::internal)?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with(&directory_prefix) || !name.ends_with(".git") {
            continue;
        }
        let path = entry.path();
        if !path.join("objects").is_dir() {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        views.push((modified, path));
    }
    views.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    Ok(views.into_iter().map(|(_, path)| path).collect())
}
