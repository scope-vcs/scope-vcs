mod objects;
mod refs;
mod requests;

use crate::{
    error::ApiError,
    git::{
        blocking,
        cache::GitRepoHandle,
        command::{git_main_oid, run_git},
        repository_git::RepositoryGit,
        request_refs::request_view_bases,
        staging::add_receive_pack_alternate,
    },
    persistence::unix_now,
    state::AppState,
};
use axum::http::HeaderMap;
use scope_domain::{requests::GitFetchAsk, views::ViewId};
use scope_postgres::db::GitFetchReader;

pub(crate) async fn prepare(
    state: &AppState,
    read: super::AuthorizedGitRead,
    view: &ViewId,
    ask: &GitFetchAsk,
) -> Result<GitRepoHandle, ApiError> {
    let super::AuthorizedGitRead {
        source,
        viewer_user_id,
    } = read;
    let config = source.repo_config.clone();
    let access = source.context.access.clone();
    let views = source.context.views.clone();
    let git = RepositoryGit::of_read_source(source);
    let reader = GitFetchReader {
        repo_id: git.incarnation.repository_id(),
        visibility_fingerprint: &scope_domain::repo_config::repo_config_fingerprint(&config)
            .map_err(ApiError::internal)?,
        view,
        user_id: viewer_user_id.as_deref(),
    };
    let selection = requests::readable_requests(state, &reader, &access, &views, ask).await?;
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
        blocking::run(move || git_main_oid(&base, "reading fetch main")).await?
    };
    let bases = request_view_bases(state, &git, &views, &selection.requests, view).await?;
    let folder = new_folder(state, vec![base.share()?]).await?;
    let objects = objects::install_objects(
        state,
        &git,
        &config,
        &reader,
        &base,
        &selection.requests,
        &folder,
    )
    .await?;
    {
        let folder = folder.share()?;
        let alternates = objects.alternates;
        blocking::run(move || {
            for alternate in alternates {
                add_receive_pack_alternate(&folder, &alternate.join("objects"))?;
            }
            Ok(())
        })
        .await?;
    }
    let folder = objects
        .leases
        .into_iter()
        .fold(folder, GitRepoHandle::with_dependency);
    let attached = refs::attach_refs(&folder, selection, bases, &main_oid).await?;
    if ask.is_listing() {
        state
            .metadata
            .repositories()
            .record_git_fetch_advertisement(&reader, &main_oid, &attached, unix_now()?)
            .await?;
    }
    Ok(folder)
}

pub(crate) async fn capabilities(
    state: &AppState,
    headers: &HeaderMap,
    owner: &str,
    repo_name: &str,
    view: &ViewId,
) -> Result<GitRepoHandle, ApiError> {
    super::authorized_git_read(state, headers, owner, repo_name, view).await?;
    new_folder(state, Vec::new()).await
}

async fn new_folder(
    state: &AppState,
    dependencies: Vec<GitRepoHandle>,
) -> Result<GitRepoHandle, ApiError> {
    let directory = tempfile::Builder::new()
        .prefix("fetch-")
        .tempdir_in(state.repository_engine.cache_root())
        .map_err(ApiError::internal)?;
    let folder = GitRepoHandle::temporary(directory, dependencies)?;
    let path = folder.share()?;
    blocking::run(move || {
        run_git(
            None,
            &[
                "init",
                "--bare",
                "--quiet",
                "--initial-branch=main",
                path.to_str()
                    .ok_or_else(|| ApiError::internal_message("invalid fetch path"))?,
            ],
            "initializing fetch folder",
        )
    })
    .await?;
    Ok(folder)
}
