use crate::{
    error::ApiError,
    git::{
        blocking,
        command::{git_stdout_text, run_git},
        pool_install::install_chain,
        pool_pack::{PoolPackRevision, stage_pool_pack},
        repository_git::RepositoryGit,
        request_refs::{bundle_prerequisites, download_snapshot, ensure_local_bare_repo},
        staging::add_receive_pack_alternate,
    },
    state::AppState,
};
use scope_domain::repository::git::{GitPoolPack, GitPoolPackOwner, RequestPackScope};
use scope_postgres::db::BundleSource;
use std::collections::BTreeMap;

pub async fn run_pool_bundle_backfill(database_url: String, dry_run: bool) -> anyhow::Result<()> {
    let state = AppState::for_bundle_backfill(database_url).await?;
    let counts = backfill(&state, dry_run).await;
    state.shutdown().await;
    let counts = counts.map_err(|e| anyhow::anyhow!(e.into_operator_diagnostic()))?;
    anyhow::ensure!(counts[2] == 0, "pool bundle verification failed");
    Ok(())
}

pub(crate) async fn backfill(state: &AppState, dry_run: bool) -> Result<[u64; 3], ApiError> {
    let store = state.metadata.repositories();
    let mut repos = BTreeMap::<String, [u64; 3]>::new();
    let mut offset = 0;
    loop {
        let revisions = store.bundle_batch(offset).await?;
        if revisions.is_empty() {
            break;
        }
        offset += revisions.len() as u64;
        for rev in revisions {
            let counts = repos.entry(rev.repo_id.clone()).or_default();
            match backfill_revision(state, &rev, dry_run).await {
                Ok(repaired) => {
                    counts[0] += 1;
                    counts[1] += u64::from(repaired);
                }
                Err(error) => {
                    counts[2] += 1;
                    let reason = error.into_operator_diagnostic();
                    println!("{} {} failed: {reason}", rev.repo_id, rev.head_oid);
                }
            }
        }
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

async fn backfill_revision(
    state: &AppState,
    rev: &BundleSource,
    dry_run: bool,
) -> Result<bool, ApiError> {
    let repos = state.metadata.repositories();
    let repo = repos
        .repository_record(&rev.repo_id)
        .await?
        .ok_or_else(|| ApiError::not_found("backfill repository missing"))?;
    let context = repos
        .git_push_state(&repo.owner_handle, &repo.name, &repo.owner_user_id)
        .await?
        .ok_or_else(|| ApiError::not_found("backfill repository missing"))?;
    let git = RepositoryGit::of_push_state(&context);
    let view = scope_domain::views::ViewId::parse(&rev.view).map_err(ApiError::internal)?;
    let base = git
        .view_repo(state, context.repo_config.views(), &view)
        .await?;
    let scope = if &view == context.repo_config.views().full() {
        RequestPackScope::Full
    } else {
        RequestPackScope::View(view)
    };
    let heads = vec![rev.head_oid.clone(), rev.old_head_oid.clone()];
    for repaired in [false, true] {
        let packs = repos.bundle_packs(&rev.repo_id, &scope, &heads).await?;
        let directory =
            tempfile::tempdir_in(state.data_dir.as_ref()).map_err(ApiError::internal)?;
        let path = directory.path().join("rev.git");
        let target = path.clone();
        let base_path = base.to_path_buf();
        git_work(state, move || {
            let target = ensure_local_bare_repo(target)?;
            add_receive_pack_alternate(&target, &base_path.join("objects"))
        })
        .await?;
        let needed = packs_supplying(packs, heads.clone());
        install_chain(state, &git.incarnation, &path, needed).await?;
        let target = path.clone();
        let wanted = heads.clone();
        let verified = git_work(state, move || {
            let args = ["fsck", "--connectivity-only", &wanted[0], &wanted[1]];
            run_git(Some(&target), &args, "verify connectivity")
        })
        .await;
        if verified.is_ok() {
            return Ok(repaired);
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
            download_snapshot(objects.as_ref(), &snapshot, &destination)?
                .map_err(ApiError::from)?;
            bundle_prerequisites(&destination)
        })
        .await?;
        let needed: Vec<_> = prerequisites.iter().chain(&heads[1..]).cloned().collect();
        let packs = repos.bundle_packs(&rev.repo_id, &scope, &needed).await?;
        let needed = packs_supplying(packs, needed);
        install_chain(state, &git.incarnation, &path, needed).await?;
        let import_path = path.clone();
        let base_path = base.to_path_buf();
        let main = git_work(state, move || {
            run_git(
                Some(&import_path),
                &["bundle", "unbundle", bundle.to_str().unwrap()],
                "import bundle",
            )?;
            let main = git_stdout_text(&base_path, &["rev-parse", "HEAD"], "read main")?;
            Ok(main.trim().to_string())
        })
        .await?;
        let mut prerequisites = vec![rev.old_head_oid.clone(), main];
        prerequisites.sort();
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
    unreachable!()
}

fn packs_supplying(packs: Vec<GitPoolPack>, mut commits: Vec<String>) -> Vec<GitPoolPack> {
    let mut remaining = packs
        .into_iter()
        .map(|p| (p.head_oid.clone(), p))
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

async fn git_work<T: Send + 'static>(
    state: &AppState,
    work: impl FnOnce() -> Result<T, ApiError> + Send + 'static,
) -> Result<T, ApiError> {
    let permit = state.runtime_budgets.wait_git_materialization().await?;
    blocking::run(move || {
        let _permit = permit;
        work()
    })
    .await
}
