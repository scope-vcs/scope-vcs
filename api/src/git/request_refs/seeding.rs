use super::{
    RequestViewBases, attach_visible_request_refs,
    pool::{request_objects_repo, sync_request_pool},
    snapshot::{BundleRef, fetch_snapshot_into_ref},
};
use crate::{
    error::ApiError,
    git::{command::git_command_output, staging::add_receive_pack_alternate},
    state::AppState,
};
use scope_domain::{
    repo_config::RepoConfig,
    repository::RepositoryIncarnation,
    requests::{Request, canonical_request_ref},
};
use std::{collections::BTreeSet, path::Path as FsPath, process::Command};

struct StoredRequestRefs<'a> {
    objects_for: &'a [Request],
    refs_for: &'a [Request],
}

pub(crate) async fn attach_request_refs_from_pool(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    config: &RepoConfig,
    objects_for: Vec<Request>,
    refs_for: Vec<Request>,
    staging_repo: &FsPath,
    view_bases: RequestViewBases,
) -> Result<Vec<Request>, ApiError> {
    let objects_repo = {
        let state = state.clone();
        let incarnation = incarnation.clone();
        let config = config.clone();
        let staging_repo = staging_repo.to_path_buf();
        crate::git::blocking::run(move || {
            let objects_repo = request_objects_repo(&state, &incarnation, &config)?;
            add_receive_pack_alternate(&staging_repo, &objects_repo.join("objects"))?;
            Ok(objects_repo)
        })
        .await?
    };
    sync_request_pool(state, incarnation, config, &objects_repo).await;
    let state = state.clone();
    let staging_repo = staging_repo.to_path_buf();
    crate::git::blocking::run(move || {
        attach_request_refs(
            &state,
            &objects_repo,
            StoredRequestRefs {
                objects_for: &objects_for,
                refs_for: &refs_for,
            },
            &staging_repo,
            &view_bases,
        )
    })
    .await
}

fn attach_request_refs(
    state: &AppState,
    objects_repo: &FsPath,
    requests: StoredRequestRefs<'_>,
    staging_repo: &FsPath,
    view_bases: &RequestViewBases,
) -> Result<Vec<Request>, ApiError> {
    let snapshotted = requests
        .objects_for
        .iter()
        .chain(requests.refs_for)
        .filter(|request| request.git_snapshot.is_some())
        .collect::<Vec<_>>();
    if !snapshotted.is_empty() {
        let present = present_commits(
            staging_repo,
            snapshotted.iter().map(|request| request.head_oid.as_str()),
        )?;
        let (shared, private): (Vec<_>, Vec<_>) = snapshotted
            .into_iter()
            .filter(|request| !present.contains(&request.head_oid))
            .partition(|request| request.is_submitted());
        for request in private {
            restore_request_objects(state, staging_repo, None, request)?;
        }
        for request in shared {
            restore_request_objects(state, objects_repo, Some(staging_repo), request)?;
        }
    }
    let (stored, snapshotless): (Vec<_>, Vec<_>) = requests
        .refs_for
        .iter()
        .cloned()
        .partition(|request| request.git_snapshot.is_some());
    update_refs(
        staging_repo,
        stored
            .iter()
            .map(|request| (canonical_request_ref(&request.name), &request.head_oid)),
    )?;
    attach_visible_request_refs(state, &snapshotless, staging_repo, view_bases)?;
    let unseeded = requests
        .objects_for
        .iter()
        .filter(|request| request.git_snapshot.is_none())
        .collect::<Vec<_>>();
    if unseeded.is_empty() {
        return Ok(Vec::new());
    }
    let reachable = present_commits(
        staging_repo,
        unseeded.iter().map(|request| request.head_oid.as_str()),
    )?;
    Ok(unseeded
        .into_iter()
        .filter(|request| !reachable.contains(&request.head_oid))
        .cloned()
        .collect())
}

fn restore_request_objects(
    state: &AppState,
    target_repo: &FsPath,
    prerequisites_from: Option<&FsPath>,
    request: &Request,
) -> Result<(), ApiError> {
    let Some(snapshot) = request.git_snapshot.as_ref() else {
        return Ok(());
    };
    let request_ref = canonical_request_ref(&request.name);
    let seeded_ref = format!("refs/scope/seeded/{}", request.name);
    let restored = fetch_snapshot_into_ref(
        state,
        target_repo,
        BundleRef {
            source: &request_ref,
            destination: &seeded_ref,
        },
        snapshot,
        prerequisites_from,
        "restoring request objects",
    );
    if present_commits(target_repo, std::iter::once(request.head_oid.as_str()))?
        .contains(&request.head_oid)
    {
        return Ok(());
    }
    restored.and(Err(ApiError::infrastructure_unavailable(
        "stored request branch snapshot does not match request metadata",
    )))
}

pub(super) fn present_commits<'a>(
    repo: &FsPath,
    oids: impl Iterator<Item = &'a str>,
) -> Result<BTreeSet<String>, ApiError> {
    if !repo.join("objects").is_dir() {
        return Ok(BTreeSet::new());
    }
    let input = oids.map(|oid| format!("{oid}\n")).collect::<String>();
    let output = git_command_output(
        Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["cat-file", "--batch-check=%(objectname) %(objecttype)"]),
        Some(input.as_bytes()),
    )?;
    let output = String::from_utf8(output).map_err(ApiError::internal)?;
    Ok(output
        .lines()
        .filter_map(|line| line.strip_suffix(" commit"))
        .map(str::to_string)
        .collect())
}

fn update_refs<'a>(
    repo: &FsPath,
    refs: impl Iterator<Item = (String, &'a String)>,
) -> Result<(), ApiError> {
    let input = refs
        .map(|(refname, oid)| format!("update {refname} {oid}\n"))
        .collect::<String>();
    if input.is_empty() {
        return Ok(());
    }
    git_command_output(
        Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["update-ref", "--stdin"]),
        Some(input.as_bytes()),
    )?;
    Ok(())
}
