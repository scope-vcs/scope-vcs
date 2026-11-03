use crate::{
    error::ApiError,
    git::{
        cache::GitRepoHandle,
        command::{git_stdout_text, run_git},
        repository_git::RepositoryGit,
        staging::add_receive_pack_alternate,
        storage::repository_storage_key,
    },
    state::AppState,
};
use scope_domain::{repo_config::RepoConfig, repository::RepositoryIncarnation, views::ViewId};
use sha2::{Digest, Sha256};

pub(crate) struct ViewObjectRepos {
    pub(crate) objects: GitRepoHandle,
    pub(crate) main: GitRepoHandle,
}

pub(crate) async fn view_object_repos(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    config: &RepoConfig,
    view: &ViewId,
) -> Result<ViewObjectRepos, ApiError> {
    view_repos(state, incarnation, config, view, None).await
}

pub(crate) async fn view_object_repo(state: &AppState, incarnation: &RepositoryIncarnation, config: &RepoConfig, view: &ViewId, reader: Option<&str>) -> Result<GitRepoHandle, ApiError> {
    Ok(view_repos(state, incarnation, config, view, reader).await?.objects)
}

async fn view_repos(state: &AppState, incarnation: &RepositoryIncarnation, config: &RepoConfig, view: &ViewId, reader: Option<&str>) -> Result<ViewObjectRepos, ApiError> {
    if view == config.views.full() || config.views.get(view).is_none() {
        return Err(ApiError::not_found("restricted view not found"));
    }
    let git = RepositoryGit::load(state, incarnation).await?;
    let projection = git.view_repo(state, config.views(), view).await?;
    let root = state.repository_engine.cache_root();
    let key = format!(
        "{}-{}-view-{}",
        repository_storage_key(incarnation),
        config.object_generation(),
        hex::encode(&Sha256::digest(view.as_str().as_bytes())[..16])
    );
    let shelf_key = reader
        .map(|reader| {
            format!(
                "{key}-fetch-{}",
                hex::encode(&Sha256::digest(reader.as_bytes())[..16])
            )
        })
        .unwrap_or_else(|| key.clone());
    let base = state
        .repository_engine
        .lease_derived(root.join(format!("{key}-projection.git")))?;
    let shelf = state
        .repository_engine
        .lease_derived(root.join(format!("{shelf_key}.git")))?
        .with_dependency(base.share()?);
    crate::git::blocking::run(move || {
        super::ensure_local_bare_repo(shelf.as_ref().to_path_buf())?;
        super::ensure_local_bare_repo(base.as_ref().to_path_buf())?;
        let head = git_stdout_text(
            &projection,
            &["rev-parse", "HEAD"],
            "reading view shelf projection head",
        )?;
        let head = head.trim();
        let retained = format!("refs/scope/projections/{head}");
        if super::request_ref_head(&base, &retained)?.as_deref() != Some(head)
            && let Err(error) = run_git(
                Some(&base),
                &[
                    "fetch",
                    "--no-tags",
                    "--no-write-fetch-head",
                    projection.to_string_lossy().as_ref(),
                    &format!("+{head}:{retained}"),
                ],
                "updating isolated view projection objects",
            )
            && super::request_ref_head(&base, &retained)?.as_deref() != Some(head)
        {
            return Err(error);
        }
        add_receive_pack_alternate(&shelf, &base.join("objects"))?;
        Ok(ViewObjectRepos {
            objects: shelf,
            main: base,
        })
    })
    .await
}
