use crate::{
    error::ApiError,
    git::{cache::GitRepoHandle, projection_repo::projection_bare_repo_for_state},
    state::AppState,
};
use scope_domain::{
    projection::project_graph, repository::Repository, requests::Request, views::ViewId,
};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) type RequestViewBases = BTreeMap<ViewId, GitRepoHandle>;

pub(crate) async fn request_view_bases(
    state: &AppState,
    repo: &Repository,
    requests: &[Request],
    seeded_view: &ViewId,
) -> Result<RequestViewBases, ApiError> {
    let views = repo.repo_config.views();
    let unseeded_views = requests
        .iter()
        .filter(|request| request.git_snapshot.is_none() && &request.view != seeded_view)
        .map(|request| &request.view)
        .filter(|view| views.get(view).is_some())
        .collect::<BTreeSet<_>>();
    let mut bases = RequestViewBases::new();
    for view in unseeded_views {
        let projection = project_graph(&repo.graph, &repo.visibility_change_sets, views, view);
        let base = projection_bare_repo_for_state(
            state,
            &repo.incarnation(),
            &projection,
            repo.git_head.as_ref(),
            &repo.git_pack_spans,
        )
        .await?;
        bases.insert(view.clone(), base);
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
