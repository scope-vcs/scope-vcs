use super::*;
use crate::git::{
    cache::GitRepoHandle,
    command::{run_git, run_git_output},
    pool_install::install_chain_for_verification,
    request_refs::packs_to_catch_up,
};
use scope_domain::repository::bundle_backfill::PoolRestoreFacts;
use scope_postgres::db::{BundleBackfillVerification, PooledRequestPack};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

pub(super) async fn git_work<T, F>(state: &AppState, work: F) -> Result<T, ApiError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, ApiError> + Send + 'static,
{
    let permit = state.runtime_budgets.wait_git_materialization().await?;
    crate::git::blocking::run(move || {
        let _permit = permit;
        work()
    })
    .await
}

pub(super) async fn empty_with_main(
    state: &AppState,
    path: PathBuf,
    base: GitRepoHandle,
) -> Result<(), ApiError> {
    git_work(state, move || {
        run_git(
            None,
            &["init", "--bare", path.to_string_lossy().as_ref()],
            "initializing pool-only restore",
        )?;
        run_git(
            Some(&path),
            &[
                "fetch",
                "--no-tags",
                "--no-write-fetch-head",
                base.to_string_lossy().as_ref(),
                "+HEAD:refs/heads/main",
            ],
            "copying only main history for pool-only restore",
        )
    })
    .await
}

pub(super) async fn listed_packs(
    state: &AppState,
    request: &Request,
    scope: &RequestPackScope,
) -> Result<Vec<PooledRequestPack>, ApiError> {
    let ids = state
        .metadata
        .requests()
        .requests_by_repo_id(&request.repo_id)
        .await?
        .into_iter()
        .filter(|candidate| candidate.view == request.view)
        .map(|candidate| candidate.id)
        .collect::<Vec<_>>();
    Ok(state
        .metadata
        .repositories()
        .git_pool_packs_for_requests(&request.repo_id, &ids)
        .await?
        .into_iter()
        .filter(|pack| &pack.scope == scope)
        .collect())
}

pub(super) async fn with_storage(
    state: &AppState,
    repo_id: &str,
    listed: Vec<PooledRequestPack>,
) -> Result<Vec<GitPoolPack>, ApiError> {
    let ids = listed
        .iter()
        .map(|pack| pack.id.clone())
        .collect::<Vec<_>>();
    let mut storage = state
        .metadata
        .repositories()
        .git_pool_pack_storage(repo_id, &ids)
        .await?
        .into_iter()
        .collect::<BTreeMap<_, _>>();
    listed
        .into_iter()
        .map(|pack| {
            let storage = storage
                .remove(&pack.id)
                .ok_or_else(|| ApiError::conflict("pool pack disappeared during backfill"))?;
            Ok(GitPoolPack {
                id: pack.id,
                owner: GitPoolPackOwner::Request {
                    request_id: pack.request_id,
                    scope: pack.scope,
                },
                head_oid: pack.head_oid,
                prerequisites: pack.prerequisites,
                storage,
            })
        })
        .collect()
}

pub(super) async fn verify(
    state: &AppState,
    claim: &BundleBackfillClaim,
    incarnation: &scope_domain::repository::RepositoryIncarnation,
    base: GitRepoHandle,
    mut listed: Vec<PooledRequestPack>,
    staged: Option<&GitPoolPack>,
    directory: &Path,
) -> Result<BundleBackfillVerification, ApiError> {
    let mut packs = with_storage(state, &claim.source.repo_id, listed.clone()).await?;
    if let Some(pack) = staged {
        let GitPoolPackOwner::Request { request_id, scope } = &pack.owner else {
            return Err(ApiError::internal_message(
                "backfill pack must belong to a request",
            ));
        };
        listed.push(PooledRequestPack {
            id: pack.id.clone(),
            request_id: request_id.clone(),
            head_oid: pack.head_oid.clone(),
            prerequisites: pack.prerequisites.clone(),
            request_view: scope
                .view()
                .cloned()
                .unwrap_or_else(scope_domain::views::ViewId::private),
            scope: scope.clone(),
            request_author_id: None,
            request_submitted: true,
        });
        packs.push(pack.clone());
    }
    let mut request_pack_ids = listed
        .iter()
        .filter(|pack| pack.request_id == claim.source.request_id)
        .map(|pack| pack.id.clone())
        .collect::<Vec<_>>();
    request_pack_ids.sort();
    let path = directory.join("verification.git");
    empty_with_main(state, path.clone(), base).await?;
    let mut request_pack_counts = BTreeMap::<String, usize>::new();
    for pack in &listed {
        *request_pack_counts
            .entry(pack.request_id.clone())
            .or_default() += 1;
    }
    let allowed = {
        let path = path.clone();
        git_work(state, move || packs_to_catch_up(&path, listed))
            .await?
            .into_iter()
            .map(|pack| pack.id)
            .collect::<std::collections::BTreeSet<_>>()
    };
    let needed = packs_supplying(
        &packs,
        vec![
            claim.source.head_oid.clone(),
            claim.source.old_head_oid.clone(),
        ],
    );
    let needed = {
        let path = path.clone();
        git_work(state, move || {
            let present = present_commits(&path, needed.iter().map(|pack| pack.head_oid.as_str()))?;
            Ok(needed
                .into_iter()
                .filter(|pack| !present.contains(&pack.head_oid))
                .collect::<Vec<_>>())
        })
        .await?
    };
    let within_reader_limits = needed.iter().all(|pack| allowed.contains(&pack.id));
    let mut fresh = state.clone();
    fresh.git_segment_store = Arc::new(
        state
            .git_segment_store
            .with_local_root(directory.join("verification-storage"))
            .map_err(|error| ApiError::infrastructure_unavailable(error.to_string()))?,
    );
    let needed_request_ids = needed
        .iter()
        .filter_map(|pack| match &pack.owner {
            GitPoolPackOwner::Request { request_id, .. } => Some(request_id.clone()),
            GitPoolPackOwner::Main { .. } => None,
        })
        .collect::<BTreeSet<_>>();
    install_chain_for_verification(&fresh, incarnation, &path, needed).await?;
    let head = claim.source.head_oid.clone();
    let old_head = claim.source.old_head_oid.clone();
    let restored = git_work(state, move || {
        let present = present_commits(&path, [head.as_str(), old_head.as_str()].into_iter())?;
        let connected = run_git_output(
            Some(&path),
            &["rev-list", "--objects", &head, &old_head],
            "checking pool-only revision connectivity",
        )?
        .status
        .success();
        PoolRestoreFacts {
            head_present: present.contains(&head),
            old_head_present: present.contains(&old_head),
            connected,
            within_reader_limits,
        }
        .verify()
        .map_err(ApiError::infrastructure_unavailable)
    })
    .await?;
    let reader_request_capacities: BTreeMap<_, _> = request_pack_counts
        .into_iter()
        .filter(|(request_id, _)| {
            request_id == &claim.source.request_id || needed_request_ids.contains(request_id)
        })
        .map(|(request_id, count)| (request_id, restored.maximum_request_pack_count(count)))
        .collect();
    let reader_packs = packs
        .into_iter()
        .filter(|pack| match &pack.owner {
            GitPoolPackOwner::Request { request_id, .. } => {
                reader_request_capacities.contains_key(request_id)
            }
            GitPoolPackOwner::Main { .. } => false,
        })
        .collect();
    Ok(BundleBackfillVerification {
        restored,
        packs: reader_packs,
        request_pack_ids,
        reader_request_capacities,
    })
}

pub(super) fn packs_supplying(packs: &[GitPoolPack], mut commits: Vec<String>) -> Vec<GitPoolPack> {
    let mut remaining = packs
        .iter()
        .map(|pack| (pack.head_oid.clone(), pack))
        .collect::<BTreeMap<_, _>>();
    let mut needed = Vec::new();
    while let Some(commit) = commits.pop() {
        if let Some(pack) = remaining.remove(&commit) {
            commits.extend(pack.prerequisites.iter().cloned());
            needed.push(pack.clone());
        }
    }
    needed
}
