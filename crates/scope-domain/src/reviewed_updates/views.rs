use crate::{
    error::DomainError,
    repo_config::RepoConfig,
    repository::Repository,
    views::{ViewId, ViewIncludes, ViewReaders, Views, ViewsTransition},
};
use std::collections::BTreeMap;

pub(super) fn views_transition(
    repo: &Repository,
    next: &RepoConfig,
    open_requests_by_view: &BTreeMap<ViewId, usize>,
) -> Result<Option<ViewsTransition>, DomainError> {
    let before = repo.repo_config.views();
    let after = next.views();
    if visibility_shape(before) == visibility_shape(after) {
        return Ok(None);
    }
    for removed in before
        .iter()
        .map(|definition| &definition.id)
        .filter(|id| after.get(id).is_none())
    {
        ensure_view_removable(repo, next, before, removed, open_requests_by_view)?;
    }
    Ok(Some(ViewsTransition {
        before: before.clone(),
        after: after.clone(),
    }))
}

fn visibility_shape(views: &Views) -> BTreeMap<&ViewId, (&ViewIncludes, &ViewReaders)> {
    views
        .iter()
        .map(|definition| (&definition.id, (&definition.includes, &definition.readers)))
        .collect()
}

fn ensure_view_removable(
    repo: &Repository,
    next: &RepoConfig,
    before: &Views,
    removed: &ViewId,
    open_requests_by_view: &BTreeMap<ViewId, usize>,
) -> Result<(), DomainError> {
    let name = before.display_name(removed);
    let refusal = if removed == before.full() {
        Some("it is the full view")
    } else if repo
        .collaboration
        .members
        .iter()
        .any(|member| &member.permissions.view == removed)
    {
        Some("members are still assigned to it")
    } else if open_requests_by_view
        .get(removed)
        .is_some_and(|count| *count > 0)
    {
        Some("requests are still open in it")
    } else if repo
        .live_files
        .keys()
        .any(|path| &repo.policy.label(path, before) == removed)
    {
        Some("files are still labelled with it")
    } else if next.files.rules.iter().any(|rule| &rule.view == removed) {
        Some("a file rule still names it")
    } else if &next.files.default == removed {
        Some("it is still the default label for new files")
    } else {
        None
    };
    match refusal {
        Some(reason) => Err(DomainError::conflict(format!(
            "view {name} cannot be removed because {reason}"
        ))),
        None => Ok(()),
    }
}
