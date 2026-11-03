use crate::{
    error::ApiError,
    git::{
        blocking,
        command::run_git,
        pool_pack::{PoolPackRevision, stage_pool_pack},
        repository_git::RepositoryGit,
        request_refs::{
            PoolInstall, PoolRepos, RequestObjectRepos, bundle_prerequisites, download_snapshot,
            ensure_local_bare_repo, install_missing_request_packs,
        },
        staging::add_receive_pack_alternate,
    },
    state::AppState,
};
use futures_util::FutureExt;
use scope_domain::repository::git::{GitPoolPackOwner, RequestPackScope};
use scope_postgres::db::BundleSource;
use std::collections::BTreeMap;

pub async fn run_pool_bundle_backfill(arguments: &[String]) -> anyhow::Result<()> {
    anyhow::ensure!(
        arguments.is_empty() || arguments == ["--dry-run"],
        "usage: scope-vcs pool-bundle-backfill [--dry-run]\nExcludes run-owned RunSource::EphemeralGitBundle and RunSource::RequestGitSnapshot bundles"
    );
    let state = AppState::for_bundle_backfill(crate::config::database_url_from_env()?).await?;
    let counts = backfill(&state, !arguments.is_empty()).boxed().await;
    state.shutdown().await;
    let counts = counts.map_err(|e| anyhow::anyhow!(e.into_operator_diagnostic()))?;
    anyhow::ensure!(counts[2] == 0, "pool bundle verification failed");
    Ok(())
}

pub(crate) async fn backfill(state: &AppState, dry_run: bool) -> Result<[u64; 3], ApiError> {
    let store = state.metadata.repositories();
    let mut repos = BTreeMap::<String, [u64; 3]>::new();
    let mut pending = Vec::new();
    loop {
        let batch = store.bundle_batch(pending.len() as u64).await?;
        if batch.is_empty() {
            break;
        }
        pending.extend(batch);
    }
    loop {
        let mut failures = Vec::new();
        let mut progress = false;
        for rev in pending {
            match backfill_revision(state, &rev, dry_run).boxed().await {
                Ok(repaired) => {
                    let counts = repos.entry(rev.repo_id).or_default();
                    counts[0] += 1;
                    counts[1] += u64::from(repaired);
                    progress |= repaired;
                }
                Err(error) => failures.push((rev, error)),
            }
        }
        if !dry_run && progress && !failures.is_empty() {
            pending = failures.into_iter().map(|(rev, _)| rev).collect();
            continue;
        }
        for (rev, error) in failures {
            repos.entry(rev.repo_id.clone()).or_default()[2] += 1;
            let reason = error.into_operator_diagnostic();
            println!("{} {} failed: {reason}", rev.repo_id, rev.head_oid);
        }
        break;
    }
    let total = repos
        .values()
        .fold([0; 3], |a, b| std::array::from_fn(|i| a[i] + b[i]));
    repos.insert("total".into(), total);
    for (repo, [verified, repaired, failed]) in repos {
        println!("{repo}: verified={verified} repaired={repaired} failed={failed}");
    }
    Ok(total)
}

pub(crate) async fn backfill_revision(
    state: &AppState,
    rev: &BundleSource,
    dry_run: bool,
) -> Result<bool, ApiError> {
    let repos = state.metadata.repositories();
    let (owner, name) = rev
        .repo_id
        .split_once('/')
        .ok_or_else(|| ApiError::internal_message("invalid repository id"))?;
    let context = repos
        .repository(owner, name)
        .await?
        .ok_or_else(|| ApiError::not_found("backfill repository missing"))?;
    let git = RepositoryGit::of_repository(&context);
    let request = state
        .metadata
        .requests()
        .request_by_id(&rev.request_id)
        .await?
        .ok_or_else(|| ApiError::not_found("backfill request missing"))?;
    let views = context.repo_config.views();
    let actor = request.author_user_id.as_deref().unwrap_or_default();
    let main = git
        .view_head(state, &request.view)
        .await?
        .ok_or_else(|| ApiError::not_found("backfill main missing"))?;
    let scope = if &request.view == views.full() {
        RequestPackScope::Full
    } else {
        RequestPackScope::View(request.view.clone())
    };
    let heads = vec![rev.head_oid.clone(), rev.old_head_oid.clone()];
    let requested = std::slice::from_ref(&request);
    let mut changed = false;
    for (read_view, head) in std::iter::once(&request.view)
        .chain((&request.view != views.full()).then_some(views.full()))
        .flat_map(|view| heads.clone().into_iter().map(move |head| (view, head)))
    {
        let read_base = git.view_repo(state, views, read_view).await?;
        for repaired in [false, true] {
            let directory =
                tempfile::tempdir_in(state.data_dir.as_ref()).map_err(ApiError::internal)?;
            let path = directory.path().join("rev.git");
            let target = path.clone();
            let base_path = read_base.to_path_buf();
            blocking::run(move || {
                let shared = ensure_local_bare_repo(target.with_extension("shared.git"))?;
                add_receive_pack_alternate(&shared, &base_path.join("objects"))?;
                let target = ensure_local_bare_repo(target)?;
                add_receive_pack_alternate(&target, &shared.join("objects"))
            })
            .await?;
            let targets = RequestObjectRepos {
                shared: path.with_extension("shared.git"),
                authored: path.clone(),
            };
            let install = PoolInstall {
                incarnation: &git.incarnation,
                config: &context.repo_config,
                repos: if read_view == views.full() {
                    PoolRepos::Full(&targets, actor, &path)
                } else {
                    PoolRepos::View(&request.view, &path)
                },
            };
            install_missing_request_packs(state, &install, requested, std::slice::from_ref(&head))
                .await?;
            let target = path.clone();
            let wanted = head.clone();
            let verified = blocking::run(move || {
                let args = ["fsck", "--connectivity-only", &wanted];
                run_git(Some(&target), &args, "verify connectivity")
            })
            .await;
            if verified.is_ok() {
                changed |= repaired;
                break;
            }
            if dry_run || repaired {
                verified?;
            }
            let bundle = directory.path().join("rev.bundle");
            let snapshot =
                serde_json::from_value(rev.git_snapshot.clone()).map_err(ApiError::internal)?;
            let objects = state.object_store.clone();
            let destination = bundle.clone();
            let prerequisites = blocking::run(move || {
                download_snapshot(objects.as_ref(), &snapshot, &destination)??;
                bundle_prerequisites(&destination)
            })
            .await?;
            let needed: Vec<_> = prerequisites.iter().chain(&heads[1..]).cloned().collect();
            install_missing_request_packs(state, &install, requested, &needed).await?;
            let target = path.clone();
            blocking::run(move || {
                run_git(
                    Some(&target),
                    &["bundle", "unbundle", bundle.to_str().unwrap()],
                    "import bundle",
                )
            })
            .await?;
            let mut prerequisites = vec![rev.old_head_oid.clone(), main.clone()];
            prerequisites.dedup();
            let pack = PoolPackRevision {
                owner: GitPoolPackOwner::Request {
                    request_id: rev.request_id.clone(),
                    scope: scope.clone(),
                },
                head_oid: rev.head_oid.clone(),
                prerequisites,
            };
            let staged = stage_pool_pack(state, &rev.repo_id, &path, pack).await?;
            let generation = context.repo_config.object_generation();
            let now = crate::persistence::unix_now()?;
            if let Err(error) = repos
                .retain_bundle_pool_pack(&git.incarnation, &generation, &staged.pack, now)
                .await
            {
                staged.discard(state, &rev.repo_id).await;
                return Err(error.into());
            }
            staged.finish(state).await;
        }
    }
    Ok(changed)
}
