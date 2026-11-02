use super::authorized_git_read;
use crate::{
    error::ApiError,
    git::{
        blocking,
        cache::GitRepoHandle,
        command::{git_command_output, git_ref_listing, git_stdout_text, run_git},
        pool_install::present_commits,
        repository_git::RepositoryGit,
        request_refs::{
            attach_readable_request_refs,
            pool::{
                PoolInstall, PoolRepos, install_request_pool_packs, request_object_repos,
                shares_objects,
            },
            request_view_bases,
            view_pool::view_object_repo_for_reader,
        },
        staging::add_receive_pack_alternate,
    },
    persistence::unix_now,
    state::AppState,
};
use axum::http::HeaderMap;
use scope_domain::{
    requests::{Request, RequestViewer, canonical_request_ref, request_policy},
    views::ViewId,
};
use scope_postgres::db::GitFetchReader;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex, OnceLock, Weak},
};

pub(crate) async fn prepare(
    state: &AppState,
    headers: &HeaderMap,
    owner: &str,
    repo_name: &str,
    view: &ViewId,
    wants: Option<&[String]>,
) -> Result<GitRepoHandle, ApiError> {
    let (source, user) = authorized_git_read(state, headers, owner, repo_name, view).await?;
    let config = source.repo_config.clone();
    let access = source.context.access.clone();
    let views = source.context.views.clone();
    let git = RepositoryGit::of_read_source(source);
    let reader = GitFetchReader {
        repo_id: git.incarnation.repository_id(),
        generation: &config.object_generation(),
        view,
        user_id: user.as_deref(),
    };
    let readable = |request: &Request, invitee: bool| {
        let decision = request_policy(
            request,
            RequestViewer::new(access.clone(), user.as_deref(), invitee),
            &views,
        );
        decision.exact_visible && views.may_read(view, &request.view)
    };
    let mut requests = state
        .metadata
        .requests()
        .git_fetch_requests(reader.repo_id, reader.user_id)
        .await?
        .into_iter()
        .filter_map(|(request, invitee)| readable(&request, invitee).then_some(request))
        .collect::<Vec<_>>();
    let parents = terminal_parents(state, &requests, user.as_deref(), &readable).await?;
    let unadvertised = parents
        .iter()
        .map(|request| request.id.clone())
        .collect::<BTreeSet<_>>();
    requests.splice(0..0, parents);
    let advertised = if let Some(wants) = wants {
        state
            .metadata
            .repositories()
            .advertised_git_fetch_heads(&reader, wants, unix_now()?)
            .await?
    } else {
        Vec::new()
    };
    let current = requests.clone();
    let mut retained = Vec::new();
    for (oid, advertised_requests) in advertised {
        let Some(advertised_requests) = advertised_requests else {
            retained.push(oid);
            continue;
        };
        let still_readable = advertised_requests
            .into_iter()
            .filter(|old| {
                current
                    .iter()
                    .any(|request| request.id == old.id && request.view == old.view)
            })
            .collect::<Vec<_>>();
        if still_readable.is_empty() {
            continue;
        }
        retained.push(oid);
        for old in still_readable {
            if !requests
                .iter()
                .any(|request| request.head_oid == old.head_oid)
            {
                requests.push(old);
            }
        }
    }
    let full = view == views.full();
    let base = match git.git_head.as_ref() {
        Some(head) if full => {
            state
                .repository_engine
                .materialize_repository(state, &git.incarnation, head)
                .await?
        }
        _ => git.view_repo(state, &views, view).await?,
    };
    let main_oid = {
        let base = base.share()?;
        blocking::run(move || {
            Ok(
                git_stdout_text(&base, &["rev-parse", "HEAD"], "reading fetch main")?
                    .trim()
                    .to_string(),
            )
        })
        .await?
    };
    let bases = request_view_bases(state, &git, &views, &requests, view).await?;
    let directory = tempfile::Builder::new()
        .prefix("fetch-")
        .tempdir_in(state.repository_engine.cache_root())
        .map_err(ApiError::internal)?;
    let folder = GitRepoHandle::temporary(directory, vec![base.share()?])?;
    {
        let folder = folder.share()?;
        blocking::run(move || {
            run_git(
                None,
                &[
                    "init",
                    "--bare",
                    "--quiet",
                    folder
                        .to_str()
                        .ok_or_else(|| ApiError::internal_message("invalid fetch path"))?,
                ],
                "initializing fetch folder",
            )?;
            run_git(
                Some(&folder),
                &["symbolic-ref", "HEAD", "refs/heads/main"],
                "setting fetch head",
            )
        })
        .await?;
    }
    let mut alternates = Vec::new();
    let mut shelves = BTreeMap::new();
    let mut guards = Vec::new();
    if full {
        let shelf_state = state.clone();
        let incarnation = git.incarnation.clone();
        let generation = config.object_generation();
        let author = user.clone().unwrap_or_default();
        let repos = blocking::run(move || {
            request_object_repos(&shelf_state, &incarnation, &generation, &author)
        })
        .await?;
        alternates.extend([
            base.as_ref().to_path_buf(),
            repos.shared.clone(),
            repos.authored.clone(),
        ]);
        let lock = shelf_lock(&repos.shared)?;
        guards.push(lock.lock_owned().await);
        install_request_pool_packs(
            state,
            &PoolInstall {
                incarnation: &git.incarnation,
                config: &config,
                repos: PoolRepos::Full {
                    repos: &repos,
                    actor_user_id: user.as_deref().unwrap_or_default(),
                },
            },
            &requests,
        )
        .await;
        for request in &requests {
            let target = if shares_objects(request, &config) {
                repos.shared.clone()
            } else if user.as_deref().is_some_and(|user| request.is_author(user)) {
                repos.authored.clone()
            } else {
                folder.as_ref().to_path_buf()
            };
            shelves.insert((request.id.clone(), request.head_oid.clone()), target);
        }
    }
    let request_views = requests
        .iter()
        .map(|request| request.view.clone())
        .chain((!full).then_some(view.clone()))
        .filter(|view| view != views.full())
        .collect::<BTreeSet<_>>();
    for request_view in request_views {
        let shelf = view_object_repo_for_reader(
            state,
            &git.incarnation,
            &config,
            &request_view,
            (!full).then_some(user.as_deref().unwrap_or_default()),
        )
        .await?;
        let subset = requests
            .iter()
            .filter(|request| {
                request.view == request_view
                    && (full
                        || request.is_submitted()
                        || user.as_deref().is_some_and(|user| request.is_author(user)))
            })
            .cloned()
            .collect::<Vec<_>>();
        let lock = shelf_lock(&shelf)?;
        guards.push(lock.lock_owned().await);
        if !full {
            validate_isolated_shelf(&shelf)?;
        }
        install_request_pool_packs(
            state,
            &PoolInstall {
                incarnation: &git.incarnation,
                config: &config,
                repos: PoolRepos::View {
                    view: &request_view,
                    repo: &shelf,
                },
            },
            &subset,
        )
        .await;
        alternates.push(shelf.clone());
        for request in subset {
            shelves
                .entry((request.id.clone(), request.head_oid.clone()))
                .or_insert_with(|| shelf.clone());
        }
    }
    {
        let folder = folder.share()?;
        let alternates = alternates.clone();
        blocking::run(move || {
            for alternate in alternates {
                add_receive_pack_alternate(&folder, &alternate.join("objects"))?;
            }
            Ok(())
        })
        .await?;
    }
    let state_for_restore = state.clone();
    let working = folder.share()?;
    let requests_for_restore = requests.clone();
    let main = main_oid.clone();
    let attached = blocking::run(move || {
        let _guards = guards;
        let present = present_commits(&working, requests_for_restore.iter().map(|request| request.head_oid.as_str()))?;
        for request in requests_for_restore.iter().filter(|request| !present.contains(&request.head_oid)) {
            let target = shelves.get(&(request.id.clone(), request.head_oid.clone())).map(PathBuf::as_path).unwrap_or(&working);
            if let Err(error) = attach_readable_request_refs(&state_for_restore, std::slice::from_ref(request), target, &bases) {
                tracing::warn!(request_id = request.id, request_name = request.name, error = %error.operator_diagnostic(), "omitting unreadable request from Git fetch");
            }
        }
        let present = present_commits(&working, requests_for_restore.iter().map(|request| request.head_oid.as_str()).chain(retained.iter().map(String::as_str)))?;
        let attached = requests_for_restore.into_iter().filter(|request| !unadvertised.contains(&request.id)).filter(|request| {
            if present.contains(&request.head_oid) { true } else {
                tracing::warn!(request_id = request.id, request_name = request.name, "omitting unavailable request from Git fetch");
                false
            }
        }).collect::<Vec<_>>();
        let mut refs = format!("update refs/heads/main {main}\n");
        let mut named = BTreeSet::new();
        for request in &attached {
            if named.insert(request.name.clone()) { refs.push_str(&format!("update {} {}\n", canonical_request_ref(&request.name), request.head_oid)); }
        }
        for (refname, _) in git_ref_listing(&working, &["refs/heads"], "reading restored fetch refs")? {
            if refname != "refs/heads/main" && !named.iter().any(|name| canonical_request_ref(name) == refname) {
                refs.push_str(&format!("delete {refname}\n"));
            }
        }
        for oid in retained {
            if present.contains(&oid) { refs.push_str(&format!("update refs/scope/fetch/{oid} {oid}\n")); }
        }
        git_command_output(Command::new("git").arg("-C").arg(working.as_ref()).args(["update-ref", "--stdin"]), Some(refs.as_bytes()))?;
        Ok(attached)
    }).await?;
    if wants.is_none() {
        state
            .metadata
            .repositories()
            .record_git_fetch_advertisement(&reader, &main_oid, &attached, unix_now()?)
            .await?;
    }
    Ok(folder)
}

async fn terminal_parents(
    state: &AppState,
    requests: &[Request],
    user: Option<&str>,
    readable: &impl Fn(&Request, bool) -> bool,
) -> Result<Vec<Request>, ApiError> {
    let store = state.metadata.requests();
    let mut known = requests
        .iter()
        .map(|request| request.id.clone())
        .collect::<BTreeSet<_>>();
    let mut pending = requests
        .iter()
        .filter(|request| request.git_snapshot.is_none())
        .filter_map(|request| request.base.parent_request_id())
        .map(str::to_string)
        .collect::<Vec<_>>();
    let mut parents = Vec::new();
    while let Some(id) = pending.pop() {
        if !known.insert(id.clone()) {
            continue;
        }
        let Some(parent) = store.request_by_id(&id).await? else {
            continue;
        };
        let invitee = match user {
            Some(user) => store.request_is_invitee(&id, user).await?,
            None => false,
        };
        if !readable(&parent, invitee) {
            continue;
        }
        if parent.git_snapshot.is_none()
            && let Some(next) = parent.base.parent_request_id()
        {
            pending.push(next.to_string());
        }
        parents.push(parent);
    }
    Ok(parents)
}

fn shelf_lock(path: &Path) -> Result<Arc<tokio::sync::Mutex<()>>, ApiError> {
    type Locks = BTreeMap<PathBuf, Weak<tokio::sync::Mutex<()>>>;
    static LOCKS: OnceLock<Mutex<Locks>> = OnceLock::new();
    let mut locks = LOCKS
        .get_or_init(Mutex::default)
        .lock()
        .map_err(|_| ApiError::internal_message("fetch shelf locks poisoned"))?;
    locks.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = locks.get(path).and_then(Weak::upgrade) {
        return Ok(lock);
    }
    let lock = Arc::new(tokio::sync::Mutex::new(()));
    locks.insert(path.to_path_buf(), Arc::downgrade(&lock));
    Ok(lock)
}

fn validate_isolated_shelf(shelf: &Path) -> Result<(), ApiError> {
    let alternates =
        fs::read_to_string(shelf.join("objects/info/alternates")).map_err(ApiError::internal)?;
    let name = shelf
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.split_once("-fetch-"))
        .map(|(name, _)| name)
        .ok_or_else(|| ApiError::internal_message("invalid restricted fetch shelf"))?;
    let expected = shelf
        .with_file_name(format!("{name}-projection.git"))
        .join("objects");
    if alternates.lines().collect::<Vec<_>>() != [expected.to_string_lossy().as_ref()]
        || expected.join("info/alternates").exists()
    {
        return Err(ApiError::internal_message(
            "restricted fetch shelf has an unauthorized alternate",
        ));
    }
    Ok(())
}
