mod restore;

use crate::{
    background_work::{BackgroundLoop, BackgroundWork, Cadence, Readiness},
    error::ApiError,
    git::{
        command::{git_stdout_text, run_git},
        pool_install::{install_chain_for_verification, present_commits},
        pool_pack::{PoolPackRevision, stage_pool_pack},
        repository_git::RepositoryGit,
        request_refs::{bundle_prerequisites, download_snapshot},
    },
    persistence_ids::generate_prefixed_id,
    state::AppState,
};
use restore::{git_work, packs_supplying};
use scope_domain::{
    repository::{
        bundle_backfill::{BACKFILL_CADENCE, BACKFILL_IDLE_CADENCE},
        git::{GitPoolPack, GitPoolPackOwner, RequestPackScope},
    },
    requests::Request,
};
use scope_postgres::db::BundleBackfillClaim;
use std::collections::{BTreeMap, BTreeSet};

type Clock<'a> = &'a (dyn Fn() -> Result<u64, ApiError> + Sync);

impl BackgroundWork {
    pub(crate) fn start_pool_bundle_backfill(&mut self) {
        self.spawn_with_activity(
            BackgroundLoop {
                name: "pool bundle backfill",
                cadence: Cadence::AfterActivity {
                    active_delay: BACKFILL_CADENCE,
                    idle_delay: BACKFILL_IDLE_CADENCE,
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
    let repositories = state.metadata.repositories();
    let token = generate_prefixed_id("pool_backfill")?;
    let Some(claim) = repositories
        .claim_pool_bundle_backfill(&token, current_time()?)
        .await?
    else {
        return Ok(false);
    };
    if let Err(error) = backfill_claim(state, &claim, current_time).await {
        tracing::warn!(source_id = claim.source.source_id, error = %error.operator_diagnostic(), "pool bundle backfill failed");
        if let Err(error) = repositories
            .fail_pool_bundle_backfill(&claim, error.operator_diagnostic(), current_time()?)
            .await
        {
            tracing::warn!(source_id = claim.source.source_id, error = %error, "pool bundle backfill failure could not be recorded");
        }
    }
    repositories
        .release_pool_bundle_backfill(claim, current_time()?)
        .await?;
    Ok(true)
}

async fn backfill_claim(
    state: &AppState,
    claim: &BundleBackfillClaim,
    current_time: Clock<'_>,
) -> Result<(), ApiError> {
    let repositories = state.metadata.repositories();
    let repo = repositories
        .repository_record(&claim.source.repo_id)
        .await?
        .ok_or_else(|| ApiError::not_found("backfill repository not found"))?;
    let config = repositories
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
    let scope = RequestPackScope::for_view(&request.view, config.views.full());
    let base = match &scope {
        RequestPackScope::Full => {
            let head = git
                .git_head
                .as_ref()
                .ok_or_else(|| ApiError::conflict("backfill repository has no accepted main"))?;
            state
                .repository_engine
                .materialize_repository(state, &incarnation, head)
                .await?
        }
        RequestPackScope::View(view) => git.view_repo(state, config.views(), view).await?,
    };
    let directory = tempfile::Builder::new()
        .prefix("pool-backfill-")
        .tempdir_in(state.data_dir.as_ref())
        .map_err(ApiError::internal)?;
    let listed = restore::listed_packs(state, &request, &scope).await?;
    let staged = if listed
        .iter()
        .any(|pack| pack.request_id == request.id && pack.head_oid == claim.source.head_oid)
    {
        None
    } else {
        let path = directory.path().join("import.git");
        restore::empty_with_main(state, path.clone(), base.share()?).await?;
        let snapshot = claim.source.snapshot()?;
        let bundle = directory.path().join("revision.bundle");
        let download_path = bundle.clone();
        let objects = state.object_store.clone();
        crate::git::blocking::run(move || {
            download_snapshot(objects.as_ref(), &snapshot, &download_path)?.map_err(ApiError::from)
        })
        .await?;
        let prerequisites = {
            let bundle = bundle.clone();
            git_work(state, move || bundle_prerequisites(&bundle)).await?
        };
        let missing = {
            let path = path.clone();
            let prerequisites = prerequisites.clone();
            git_work(state, move || {
                let present = present_commits(&path, prerequisites.iter().map(String::as_str))?;
                Ok(prerequisites
                    .into_iter()
                    .filter(|oid| !present.contains(oid))
                    .collect::<Vec<_>>())
            })
            .await?
        };
        let mut supply = missing;
        supply.push(claim.source.old_head_oid.clone());
        let packs = restore::with_storage(state, &request.repo_id, listed.clone()).await?;
        let needed_packs = packs_supplying(&packs, supply);
        drop(packs);
        install_chain_for_verification(state, &incarnation, &path, needed_packs).await?;
        let import_path = path.clone();
        let head = claim.source.head_oid.clone();
        let old_head = claim.source.old_head_oid.clone();
        let prerequisites = git_work(state, move || {
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
            let main = git_stdout_text(
                &import_path,
                &["rev-parse", "refs/heads/main"],
                "reading backfill view main",
            )?;
            let candidates = prerequisites
                .into_iter()
                .chain([old_head, main.trim().to_string()])
                .filter(|oid| oid != &head)
                .collect::<BTreeSet<_>>();
            Ok(
                present_commits(&import_path, candidates.iter().map(String::as_str))?
                    .into_iter()
                    .collect::<Vec<_>>(),
            )
        })
        .await?;
        Some(
            stage_pool_pack(
                state,
                &request.repo_id,
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
            .await?,
        )
    };
    let result = async {
        let verification = restore::verify(
            state,
            claim,
            &incarnation,
            base,
            listed,
            staged.as_ref().map(|staged| &staged.pack),
            directory.path(),
        )
        .await?;
        repositories
            .complete_pool_bundle_backfill(
                claim,
                &incarnation,
                &config.object_generation(),
                staged.as_ref().map(|staged| &staged.pack),
                &verification,
                current_time()?,
            )
            .await
            .map_err(ApiError::from)
    }
    .await;
    if let Some(staged) = staged {
        if matches!(result, Ok(true)) {
            staged.finish(state).await;
        } else {
            staged.discard(state, incarnation.repository_id()).await;
        }
    }
    result.map(drop)
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
        let mut heads = packs_supplying(&packs, vec!["second".to_string()])
            .into_iter()
            .map(|pack| pack.head_oid)
            .collect::<Vec<_>>();
        heads.sort();
        assert_eq!(heads, ["first", "second"]);
    }
}
