use crate::{
    error::ApiError,
    git::{
        command::{git_stdout_text, run_git},
        repository_git::RepositoryGit,
        staging::add_receive_pack_alternate,
        storage::request_objects_root,
    },
    state::AppState,
};
use scope_domain::{repo_config::RepoConfig, repository::RepositoryIncarnation, views::ViewId};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

pub(crate) async fn view_object_repo(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    config: &RepoConfig,
    view: &ViewId,
    request_id: Option<&str>,
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
    let shelf_key = match request_id {
        Some(id) => format!(
            "{key}-request-{}",
            hex::encode(&Sha256::digest(id.as_bytes())[..16])
        ),
        None => key.clone(),
    };
    crate::git::blocking::run(move || {
        let head = git_stdout_text(
            &projection,
            &["rev-parse", "HEAD"],
            "reading view shelf projection head",
        )?;
        let head = head.trim();
        let base = super::ensure_local_bare_repo(root.join(format!("{key}-projection.git")))?;
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
        let shelf = super::ensure_local_bare_repo(root.join(format!("{shelf_key}.git")))?;
        add_receive_pack_alternate(&shelf, &base.join("objects"))?;
        Ok(shelf)
    })
    .await
}
