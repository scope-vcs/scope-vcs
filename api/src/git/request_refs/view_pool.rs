use crate::{
    error::ApiError,
    git::{
        command::git_stdout_text, repository_git::RepositoryGit,
        staging::add_receive_pack_alternate, storage::request_objects_root,
    },
    state::AppState,
};
use scope_domain::{repo_config::RepoConfig, repository::RepositoryIncarnation, views::ViewId};
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};

pub(crate) async fn view_object_repo(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    config: &RepoConfig,
    view: &ViewId,
) -> Result<PathBuf, ApiError> {
    if view == config.views.full() || config.views.get(view).is_none() {
        return Err(ApiError::not_found("restricted view not found"));
    }
    let git = RepositoryGit::load(state, incarnation).await?;
    let projection = git.view_repo(state, config.views(), view).await?;
    let root = request_objects_root(state, incarnation);
    let key = format!(
        "{}-view-{}",
        config.object_generation(),
        hex::encode(&Sha256::digest(view.as_str().as_bytes())[..16])
    );
    crate::git::blocking::run(move || {
        let shelf = super::ensure_local_bare_repo(root.join(format!("{key}.git")))?;
        let head = git_stdout_text(
            &projection,
            &["rev-parse", "HEAD"],
            "reading view shelf projection head",
        )?;
        let base = root.join(format!("{key}-{}.git", head.trim()));
        if !base.join("objects").is_dir() {
            let temporary = tempfile::Builder::new()
                .prefix(".view-base-")
                .tempdir_in(&root)
                .map_err(ApiError::internal)?;
            let prepared = temporary.path().join("projection.git");
            super::clone_without_shared_objects(
                &projection,
                &prepared,
                "isolating view shelf projection objects",
            )?;
            if let Err(error) = fs::rename(&prepared, &base)
                && !base.join("objects").is_dir()
            {
                return Err(ApiError::internal(error));
            }
        }
        add_receive_pack_alternate(&shelf, &base.join("objects"))?;
        Ok(shelf)
    })
    .await
}
