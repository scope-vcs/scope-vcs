use crate::{
    error::ApiError,
    git::{
        blocking::{block_on as wait, run},
        command::run_git,
        pool_pack::{PoolPackRevision, StagedPoolPack, stage_pool_pack},
        repository_git::RepositoryGit,
        request_refs::{bundle_prerequisites, download_snapshot},
    },
    state::AppState,
};
use scope_domain::repository::git::{GitPoolPackOwner, RequestPackScope};
use scope_domain::requests::request_visible_to_every_maintainer as shared;
use scope_postgres::db::BundleSource;

pub(crate) mod reader;
use reader::PoolBundleReader;

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
        let mut staged: Option<StagedPoolPack> = None;
        let mut retained = false;
        let result = (|| {
            'verify: for repair in [!dry_run, false, false] {
                for (read_view, head) in std::iter::once(&request.view)
                    .chain((&request.view != views.full()).then_some(views.full()))
                    .flat_map(|view| heads.clone().into_iter().map(move |head| (view, head)))
                {
                    let mut reader = PoolBundleReader::new(
                        &state,
                        &git,
                        &context.repo_config,
                        actor,
                        read_view,
                    )?;
                    if let Some(candidate) = &staged
                        && !retained
                        && (read_view != views.full() || allowed)
                    {
                        reader.lease_candidate_base(&state, &candidate.pack)?;
                    }
                    reader.install(
                        &state,
                        std::slice::from_ref(&request),
                        std::slice::from_ref(&head),
                    )?;
                    if let Some(StagedPoolPack { pack, .. }) = &staged
                        && !retained
                        && pack.head_oid == head
                        && (read_view != views.full() || allowed)
                    {
                        reader.install_candidate(&state, &request, pack)?;
                    }
                    let verified = reader.verify(&head);
                    if verified.is_ok() {
                        continue;
                    }
                    if !repair {
                        verified?;
                    }
                    let bundle = reader.directory.path().join("rev.bundle");
                    let snapshot = serde_json::from_value(rev.git_snapshot.clone())
                        .map_err(ApiError::internal)?;
                    download_snapshot(state.object_store.as_ref(), &snapshot, &bundle)?;
                    let prerequisites = bundle_prerequisites(&bundle)?;
                    let needed: Vec<_> = prerequisites.iter().chain(&heads[1..]).cloned().collect();
                    reader.install(&state, std::slice::from_ref(&request), &needed)?;
                    let import = ["bundle", "unbundle", bundle.to_str().unwrap()];
                    run_git(Some(&reader.path), &import, "import bundle")?;
                    let pack = PoolPackRevision {
                        owner: GitPoolPackOwner::Request {
                            request_id: rev.request_id.clone(),
                            scope: scope.clone(),
                        },
                        head_oid: rev.head_oid.clone(),
                        prerequisites: needed,
                    };
                    staged = Some(wait(stage_pool_pack(
                        &state,
                        &rev.repo_id,
                        &reader.path,
                        pack,
                    ))?);
                    continue 'verify;
                }
                if let Some(candidate) = &staged
                    && !retained
                {
                    let now = crate::persistence::unix_now()?;
                    wait(repos.retain_bundle_pool_pack(&context, &candidate.pack, now))?;
                    retained = true;
                    continue 'verify;
                }
                return Ok::<_, ApiError>(());
            }
            unreachable!()
        })();
        let changed = staged.is_some();
        if let Some(staged) = staged {
            if result.is_err() && !retained {
                wait(staged.discard(&state, &rev.repo_id));
            } else {
                wait(staged.finish(&state));
            }
        }
        result.map(|()| changed)
    })
    .await
}
