use crate::{
    error::ApiError,
    git::{
        blocking::{block_on as wait, run},
        command::run_git,
        pool_install::install_chain,
        pool_pack::{PoolPackRevision, StagedPoolPack, stage_pool_pack},
        repository_git::RepositoryGit,
        request_refs::{
            PoolInstall, PoolRepos, RequestObjectRepos, bundle_prerequisites, download_snapshot,
            ensure_local_bare_repo, install_missing_request_packs as install_packs,
        },
        staging::add_receive_pack_alternate,
    },
    state::AppState,
};
use scope_domain::repository::git::{GitPoolPackOwner, RequestPackScope};
use scope_domain::requests::request_visible_to_every_maintainer as shared;
use scope_postgres::db::BundleSource;

pub async fn run_pool_bundle_backfill(arguments: &[String]) -> anyhow::Result<()> {
    let flags = ["--dry-run", "--allow-empty"];
    anyhow::ensure!(
        arguments.iter().all(|arg| flags.contains(&arg.as_str())),
        "usage: scope-vcs pool-bundle-backfill [--dry-run] [--allow-empty]"
    );
    let database = url::Url::parse(&crate::config::database_url_from_env()?)?;
    let state = AppState::for_bundle_backfill(database.to_string()).await?;
    let counts = backfill(&state, arguments.iter().any(|arg| arg == "--dry-run")).await;
    state.shutdown().await;
    let counts = counts.map_err(|e| anyhow::anyhow!(e.into_operator_diagnostic()))?;
    let host = database.host_str().unwrap_or("unix-socket");
    println!("database_host={host} scanned={}", counts[0] + counts[2]);
    validate_backfill(counts, arguments.iter().any(|arg| arg == "--allow-empty"))
}

pub(crate) fn validate_backfill(counts: [u64; 3], allow_empty: bool) -> anyhow::Result<()> {
    anyhow::ensure!(
        (counts[0] + counts[2] > 0 || allow_empty) && counts[2] == 0,
        "verification failed or no revisions scanned; check database host or pass --allow-empty"
    );
    Ok(())
}

pub(crate) async fn backfill(state: &AppState, dry_run: bool) -> Result<[u64; 3], ApiError> {
    let store = state.metadata.repositories();
    let mut counts = [0; 3];
    let mut pending = store.bundle_batch(None, None).await?;
    let keys: Vec<_> = pending.iter().map(|rev| rev.source_id.clone()).collect();
    pending.sort_by_key(|rev| (rev.stack_depth, rev.position));
    loop {
        let mut failures = Vec::new();
        let mut progress = false;
        for rev in pending {
            match backfill_revision(state, &rev, dry_run).await {
                Ok(repaired) => {
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
            counts[2] += 1;
            let reason = error.into_operator_diagnostic();
            println!("{} {} failed: {reason}", rev.repo_id, rev.head_oid);
        }
        break;
    }
    let [verified, repaired, failed] = counts;
    println!("total: verified={verified} repaired={repaired} failed={failed}");
    let current = store.bundle_batch(None, None).await?;
    if !keys.iter().eq(current.iter().map(|rev| &rev.source_id)) {
        return Err(ApiError::conflict("revisions changed; stop writers; rerun"));
    }
    Ok(counts)
}

pub(crate) async fn backfill_revision(
    state: &AppState,
    rev: &BundleSource,
    dry_run: bool,
) -> Result<bool, ApiError> {
    let state = state.clone();
    let rev = rev.clone();
    run(move || {
        let repos = state.metadata.repositories();
        let (owner, name) = rev
            .repo_id
            .split_once('/')
            .ok_or_else(|| ApiError::internal_message("invalid repository id"))?;
        let context = wait(repos.repository(owner, name))?
            .ok_or_else(|| ApiError::not_found("backfill repository missing"))?;
        let git = RepositoryGit::of_repository(&context);
        let request = wait(state.metadata.requests().request_by_id(&rev.request_id))?
            .ok_or_else(|| ApiError::not_found("backfill request missing"))?;
        let views = context.repo_config.views();
        let actor = request.author_user_id.as_deref().unwrap_or_default();
        let allowed = !actor.is_empty() || shared(&request.view, request.is_submitted(), views);
        let scope = if &request.view == views.full() {
            RequestPackScope::Full
        } else {
            RequestPackScope::View(request.view.clone())
        };
        let heads = [rev.head_oid.clone(), rev.old_head_oid.clone()];
        let requested = std::slice::from_ref(&request);
        let incarnation = &git.incarnation;
        let mut staged: Option<StagedPoolPack> = None;
        let result = (|| {
            'verify: for repair in [!dry_run, false] {
                for (read_view, head) in std::iter::once(&request.view)
                    .chain((&request.view != views.full()).then_some(views.full()))
                    .flat_map(|view| heads.clone().into_iter().map(move |head| (view, head)))
                {
                    let directory = tempfile::tempdir_in(state.data_dir.as_ref())
                        .map_err(ApiError::internal)?;
                    let path = ensure_local_bare_repo(directory.path().join("rev.git"))?;
                    for view in [read_view, &request.view] {
                        let base = wait(git.view_repo(&state, views, view))?;
                        add_receive_pack_alternate(&path, &base.join("objects"))?;
                    }
                    let targets = RequestObjectRepos {
                        shared: path.clone(),
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
                    let install =
                        |heads: &[String]| wait(install_packs(&state, &install, requested, heads));
                    install(std::slice::from_ref(&head))?;
                    if let Some(StagedPoolPack { pack, .. }) = &staged
                        && pack.head_oid == head
                        && (read_view != views.full() || allowed)
                    {
                        install(&pack.prerequisites)?;
                        let packs = vec![pack.clone()];
                        wait(install_chain(&state, incarnation, &path, packs))?;
                    }
                    let check = ["fsck", "--connectivity-only", &head];
                    let verified = run_git(Some(&path), &check, "verify connectivity");
                    if verified.is_ok() {
                        continue;
                    }
                    if !repair {
                        verified?;
                    }
                    let bundle = directory.path().join("rev.bundle");
                    let snapshot = serde_json::from_value(rev.git_snapshot.clone())
                        .map_err(ApiError::internal)?;
                    download_snapshot(state.object_store.as_ref(), &snapshot, &bundle)?;
                    let prerequisites = bundle_prerequisites(&bundle)?;
                    let needed: Vec<_> = prerequisites.iter().chain(&heads[1..]).cloned().collect();
                    install(&needed)?;
                    let import = ["bundle", "unbundle", bundle.to_str().unwrap()];
                    run_git(Some(&path), &import, "import bundle")?;
                    let pack = PoolPackRevision {
                        owner: GitPoolPackOwner::Request {
                            request_id: rev.request_id.clone(),
                            scope: scope.clone(),
                        },
                        head_oid: rev.head_oid.clone(),
                        prerequisites: needed,
                    };
                    staged = Some(wait(stage_pool_pack(&state, &rev.repo_id, &path, pack))?);
                    continue 'verify;
                }
                if let Some(candidate) = &staged {
                    let now = crate::persistence::unix_now()?;
                    wait(repos.retain_bundle_pool_pack(&context, &candidate.pack, now))?;
                }
                return Ok::<_, ApiError>(());
            }
            unreachable!()
        })();
        let changed = staged.is_some();
        if let Some(staged) = staged {
            if result.is_err() {
                wait(staged.discard(&state, &rev.repo_id));
            } else {
                wait(staged.finish(&state));
            }
        }
        result.map(|()| changed)
    })
    .await
}
