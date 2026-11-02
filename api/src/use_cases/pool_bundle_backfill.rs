use crate::{
    background_work::{BackgroundLoop, BackgroundWork, Cadence, Readiness},
    error::ApiError,
    git::{
        command::{git_stdout_text, run_git},
        pool_install::{install_chain, present_commits},
        pool_pack::{PoolPackRevision, StagedPoolPack, stage_pool_pack},
        repository_git::RepositoryGit,
        request_refs::{bundle_prerequisites, download_snapshot},
        staging::add_receive_pack_alternate,
    },
    persistence_ids::generate_prefixed_id,
    state::AppState,
};
use scope_domain::{
    repository::git::{GitPoolPack, GitPoolPackOwner, RequestPackScope},
    requests::Request,
};
use scope_postgres::db::BundleBackfillClaim;
use std::{collections::BTreeMap, time::Duration};

type Clock<'a> = &'a (dyn Fn() -> Result<u64, ApiError> + Sync);

impl BackgroundWork {
    pub(crate) fn start_pool_bundle_backfill(&mut self) {
        self.spawn(
            BackgroundLoop {
                name: "pool bundle backfill",
                cadence: Cadence::AfterPass {
                    delay: Duration::from_secs(30),
                    wakeup: None,
                },
                readiness: Readiness::SchemaRequired,
            },
            |state| async move { backfill_one_bundle(&state, &crate::persistence::unix_now).await },
        );
    }
}

pub(crate) async fn backfill_one_bundle(
    state: &AppState,
    current_time: Clock<'_>,
) -> Result<bool, ApiError> {
    let now_unix = current_time()?;
    let repositories = state.metadata.repositories();
    let token = generate_prefixed_id("pool_backfill")?;
    let Some(claim) = repositories
        .claim_pool_bundle_backfill(&token, now_unix)
        .await?
    else {
        return Ok(false);
    };
    let recorded = match backfill_claim(state, &claim, current_time).await {
        Ok(recorded) => recorded,
        Err(error) => {
            tracing::warn!(source_id = claim.source.source_id, error = %error.operator_diagnostic(), "pool bundle backfill failed");
            repositories
                .fail_pool_bundle_backfill(&claim, error.operator_diagnostic(), current_time()?)
                .await?;
            false
        }
    };
    repositories
        .release_pool_bundle_backfill(claim, current_time()?)
        .await?;
    Ok(recorded)
}

async fn backfill_claim(
    state: &AppState,
    claim: &BundleBackfillClaim,
    current_time: Clock<'_>,
) -> Result<bool, ApiError> {
    let repo = state
        .metadata
        .repositories()
        .repository_record(&claim.source.repo_id)
        .await?
        .ok_or_else(|| ApiError::not_found("backfill repository not found"))?;
    let config = state
        .metadata
        .repositories()
        .git_push_state(&repo.owner_handle, &repo.name, &repo.owner_user_id)
        .await?
        .ok_or_else(|| ApiError::not_found("backfill repository not found"))?
        .repo_config;
    let request = state
        .metadata
        .requests()
        .request_by_id(&claim.source.request_id)
        .await?
        .ok_or_else(|| ApiError::not_found("backfill request not found"))?;
    let incarnation = repo.incarnation();
    let git = RepositoryGit::load(state, &incarnation).await?;
    let base = git.view_repo(state, config.views(), &request.view).await?;
    let directory = tempfile::Builder::new()
        .prefix("pool-backfill-")
        .tempdir_in(state.data_dir.as_ref())
        .map_err(ApiError::internal)?;
    let path = directory.path().join("revision.git");
    let snapshot = claim.source.snapshot()?;
    let state_for_import = state.clone();
    let import_path = path.clone();
    let permit = state.runtime_budgets.wait_git_materialization().await?;
    let (directory, prerequisites, _base, _permit) = crate::git::blocking::run(move || {
        run_git(
            None,
            &["init", "--bare", import_path.to_string_lossy().as_ref()],
            "initializing bundle backfill",
        )?;
        add_receive_pack_alternate(&import_path, &base.join("objects"))?;
        let bundle = directory.path().join("revision.bundle");
        download_snapshot(state_for_import.object_store.as_ref(), &snapshot, &bundle)?
            .map_err(ApiError::from)?;
        let prerequisites = bundle_prerequisites(&bundle)?;
        Ok((directory, prerequisites, base, permit))
    })
    .await?;
    let scope = if request.view == *config.views.full() {
        RequestPackScope::Full
    } else {
        RequestPackScope::View(request.view.clone())
    };
    let missing = {
        let path = path.clone();
        let prerequisites = prerequisites.clone();
        crate::git::blocking::run(move || {
            let present = present_commits(&path, prerequisites.iter().map(String::as_str))?;
            Ok(prerequisites
                .into_iter()
                .filter(|oid| !present.contains(oid))
                .collect::<Vec<_>>())
        })
        .await?
    };
    if !missing.is_empty() {
        let packs = request_packs(state, &request, &scope).await?;
        let packs = packs_supplying(packs, missing);
        install_chain(state, &incarnation, &path, packs).await?;
    }
    let import_path = path.clone();
    let head = claim.source.head_oid.clone();
    let directory = crate::git::blocking::run(move || {
        let bundle = directory.path().join("revision.bundle");
        run_git(
            Some(&import_path),
            &["bundle", "verify", bundle.to_string_lossy().as_ref()],
            "verifying backfill bundle prerequisites",
        )?;
        let heads = git_stdout_text(
            &import_path,
            &["bundle", "list-heads", bundle.to_string_lossy().as_ref()],
            "reading backfill bundle heads",
        )?;
        if !heads
            .lines()
            .any(|line| line.split_ascii_whitespace().next() == Some(head.as_str()))
        {
            return Err(ApiError::infrastructure_unavailable(
                "bundle does not advertise its recorded revision head",
            ));
        }
        run_git(
            Some(&import_path),
            &["bundle", "unbundle", bundle.to_string_lossy().as_ref()],
            "unbundling backfill revision",
        )?;
        let actual = git_stdout_text(
            &import_path,
            &["rev-parse", "--verify", &format!("{head}^{{commit}}")],
            "verifying backfill revision head",
        )?;
        if actual.trim() != head {
            return Err(ApiError::infrastructure_unavailable(
                "bundle does not contain its recorded revision head",
            ));
        }
        Ok(directory)
    })
    .await?;
    let staged = stage_pool_pack(
        state,
        &claim.source.repo_id,
        &path,
        PoolPackRevision {
            owner: GitPoolPackOwner::Request {
                request_id: request.id.clone(),
                scope,
            },
            head_oid: claim.source.head_oid.clone(),
            prerequisites,
        },
    )
    .await?;
    let result = persist_staged(
        state,
        claim,
        &incarnation,
        &config.object_generation(),
        staged,
        current_time()?,
    )
    .await;
    drop(directory);
    result
}

async fn persist_staged(
    state: &AppState,
    claim: &BundleBackfillClaim,
    incarnation: &scope_domain::repository::RepositoryIncarnation,
    generation: &str,
    staged: StagedPoolPack,
    now_unix: u64,
) -> Result<bool, ApiError> {
    match state
        .metadata
        .repositories()
        .complete_pool_bundle_backfill(claim, incarnation, generation, &staged.pack, now_unix)
        .await
    {
        Ok(true) => {
            staged.finish(state).await;
            Ok(true)
        }
        Ok(false) => {
            staged.discard(state, incarnation.repository_id()).await;
            Ok(false)
        }
        Err(error) => {
            staged.discard(state, incarnation.repository_id()).await;
            Err(error.into())
        }
    }
}

fn packs_supplying(packs: Vec<GitPoolPack>, mut commits: Vec<String>) -> Vec<GitPoolPack> {
    let mut remaining = packs
        .into_iter()
        .map(|pack| (pack.head_oid.clone(), pack))
        .collect::<BTreeMap<_, _>>();
    let mut needed = Vec::new();
    while let Some(commit) = commits.pop() {
        if let Some(pack) = remaining.remove(&commit) {
            commits.extend(pack.prerequisites.iter().cloned());
            needed.push(pack);
        }
    }
    needed
}

async fn request_packs(
    state: &AppState,
    request: &Request,
    scope: &RequestPackScope,
) -> Result<Vec<GitPoolPack>, ApiError> {
    let listed = state
        .metadata
        .repositories()
        .git_pool_packs_for_requests(&request.repo_id, std::slice::from_ref(&request.id))
        .await?;
    let ids = listed
        .iter()
        .map(|pack| pack.id.clone())
        .collect::<Vec<_>>();
    let mut storage = state
        .metadata
        .repositories()
        .git_pool_pack_storage(&request.repo_id, &ids)
        .await?
        .into_iter()
        .collect::<BTreeMap<_, _>>();
    Ok(listed
        .into_iter()
        .filter(|pack| &pack.scope == scope)
        .filter_map(|pack| {
            storage.remove(&pack.id).map(|storage| GitPoolPack {
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
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use scope_domain::repository::git::GitPoolPackStorage;

    fn pack(head: &str, prerequisites: &[&str]) -> GitPoolPack {
        GitPoolPack {
            id: head.to_string(),
            owner: GitPoolPackOwner::Request {
                request_id: "request".to_string(),
                scope: RequestPackScope::Full,
            },
            head_oid: head.to_string(),
            prerequisites: prerequisites.iter().map(ToString::to_string).collect(),
            storage: GitPoolPackStorage::Inline { sealed: Vec::new() },
        }
    }

    #[test]
    fn only_the_chain_behind_a_missing_commit_loads() {
        let packs = vec![
            pack("first", &["main"]),
            pack("second", &["first"]),
            pack("unrelated", &["main"]),
        ];
        let mut heads = packs_supplying(packs, vec!["second".to_string()])
            .into_iter()
            .map(|pack| pack.head_oid)
            .collect::<Vec<_>>();
        heads.sort();
        assert_eq!(heads, ["first", "second"]);
    }
}
