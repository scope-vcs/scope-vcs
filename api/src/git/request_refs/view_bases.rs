use crate::{
    error::ApiError,
    git::{cache::GitRepoHandle, repository_git::RepositoryGit},
    state::AppState,
};
use scope_domain::{
    requests::Request,
    views::{ViewId, Views},
};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) type RequestViewBases = BTreeMap<ViewId, GitRepoHandle>;

pub(crate) async fn request_view_bases(
    state: &AppState,
    git: &RepositoryGit,
    views: &Views,
    requests: &[Request],
    seeded_view: &ViewId,
) -> Result<RequestViewBases, ApiError> {
    let unseeded_views = requests
        .iter()
        .filter(|request| request.git_snapshot.is_none() && &request.view != seeded_view)
        .map(|request| &request.view)
        .filter(|view| views.get(view).is_some())
        .collect::<BTreeSet<_>>();
    let mut bases = RequestViewBases::new();
    for view in unseeded_views {
        bases.insert(view.clone(), git.view_repo(state, views, view).await?);
    }
    Ok(bases)
}

pub(crate) fn share_request_view_bases(
    bases: &RequestViewBases,
) -> Result<RequestViewBases, ApiError> {
    bases
        .iter()
        .map(|(view, base)| Ok((view.clone(), base.share()?)))
        .collect()
}
