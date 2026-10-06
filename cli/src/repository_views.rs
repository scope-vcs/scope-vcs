use crate::{
    api::{ApiSession, RepoSummaryResponse, ViewDefinition, get_repo_config},
    repo_config::default_scope_repo_config,
};
use anyhow::Context;
use scope_domain::{repo_config::RepoConfig, views::Views};

pub fn repository_views(definitions: &[ViewDefinition]) -> anyhow::Result<Views> {
    serde_json::to_value(definitions)
        .and_then(serde_json::from_value)
        .context("validate repository views")
}

pub fn reads_full_view(summary: &RepoSummaryResponse) -> anyhow::Result<bool> {
    let views = repository_views(&summary.views)?;
    Ok(views.may_read(&summary.access.view.clone().into(), views.full()))
}

pub fn reader_repo_config(
    api: ApiSession<'_>,
    owner: &str,
    repo: &str,
    summary: &RepoSummaryResponse,
) -> anyhow::Result<RepoConfig> {
    if reads_full_view(summary)? {
        return Ok(get_repo_config(api, owner, repo)?.config);
    }
    let mut config = default_scope_repo_config();
    config.views = repository_views(&summary.views)?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::ViewId;

    fn summary(view: &str, definitions: serde_json::Value) -> RepoSummaryResponse {
        serde_json::from_value(serde_json::json!({
            "description": null,
            "website_url": null,
            "id": "repo_one",
            "owner_handle": "owner",
            "name": "repo",
            "git_remote_url": "https://scope.example/git/private/owner/repo",
            "lifecycle_state": "Ready",
            "change_version": 1,
            "content_version": 1,
            "access": {
                "actor": "Member",
                "view": view,
                "can_push": false,
                "can_change_file_visibility": false,
                "can_manage_members": false,
                "can_delete_repo": false,
            },
            "views": definitions,
            "open_request_count": 0,
        }))
        .unwrap()
    }

    fn agent_views() -> serde_json::Value {
        serde_json::json!([
            {"id": "public", "name": "Public", "includes": [], "readers": "anyone"},
            {"id": "private", "name": "Private", "includes": "all", "readers": "assigned"},
            {"id": "agent", "name": "Agent", "includes": ["public"], "readers": "assigned"},
        ])
    }

    #[test]
    fn only_readers_of_the_full_view_read_the_repository_config() {
        assert!(reads_full_view(&summary("private", agent_views())).unwrap());
        assert!(!reads_full_view(&summary("agent", agent_views())).unwrap());
        assert!(!reads_full_view(&summary("public", agent_views())).unwrap());
    }

    #[test]
    fn repository_views_keep_custom_names_and_reject_invalid_definitions() {
        let views = repository_views(&summary("agent", agent_views()).views).unwrap();
        assert_eq!(
            views.display_name(&ViewId::parse("agent").unwrap().into()),
            "Agent"
        );
        let invalid = serde_json::json!([
            {"id": "private", "name": "Private", "includes": "all", "readers": "assigned"},
            {"id": "agent", "name": "Agent", "includes": ["missing"], "readers": "assigned"},
        ]);
        assert!(repository_views(&summary("agent", invalid).views).is_err());
    }
}
