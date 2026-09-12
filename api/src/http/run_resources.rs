use crate::{
    auth::scope::require_scope_user,
    error::ApiError,
    http::{
        responses::{
            RepositoryRunHistoryPageResponse, RepositoryRunWorkflowListResponse,
            RepositoryRunWorkflowResponse,
        },
        run_response::repository_run_summary,
    },
    state::AppState,
    use_cases::run_inspection::require_repo_member,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use scope_domain::runs::workflow::revision::WorkflowRevision;
use scope_postgres::db::{RunHistoryCursor, RunHistoryPageQuery};
use serde::Deserialize;

const DEFAULT_RUN_HISTORY_PAGE_SIZE: usize = 20;
const MAX_RUN_HISTORY_PAGE_SIZE: usize = 100;

#[derive(Debug, Deserialize)]
pub(crate) struct RepositoryRunHistoryQuery {
    workflow: Option<String>,
    git_oid: Option<String>,
    after: Option<String>,
    limit: Option<usize>,
}

pub(crate) async fn get_repository_run_workflows(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<RepositoryRunWorkflowListResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let repo = require_repo_member(&state, &user.id, &owner, &repo_name).await?;
    let workflows = current_workflows(&state, &repo.record.id)
        .await?
        .into_iter()
        .map(|revision| RepositoryRunWorkflowResponse {
            key: revision.workflow().path().name().to_string(),
            name: revision.definition().name().to_string(),
            path: revision.workflow().path().as_str().to_string(),
            manual: revision.definition().triggers().manual(),
            push_main: revision.definition().triggers().push_main(),
            job_count: revision.definition().jobs().len(),
        })
        .collect();
    Ok(Json(RepositoryRunWorkflowListResponse { workflows }))
}

pub(crate) async fn get_repository_run_history(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
    Query(query): Query<RepositoryRunHistoryQuery>,
) -> Result<Json<RepositoryRunHistoryPageResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let repo = require_repo_member(&state, &user.id, &owner, &repo_name).await?;
    let workflow = query
        .workflow
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty());
    let workflow_path = if let Some(key) = workflow {
        Some(
            current_workflows(&state, &repo.record.id)
                .await?
                .into_iter()
                .find(|revision| revision.workflow().path().name() == key)
                .map(|revision| revision.workflow().path().as_str().to_string())
                .ok_or_else(|| ApiError::bad_request("workflow is not defined on current main"))?,
        )
    } else {
        None
    };
    let git_oid = query
        .git_oid
        .as_deref()
        .map(|value| crate::http::responses::git_oid_request("git_oid", value))
        .transpose()?;
    let after = query
        .after
        .as_deref()
        .map(|value| parse_history_cursor(value, workflow, git_oid.as_deref()))
        .transpose()?;
    let limit = query
        .limit
        .unwrap_or(DEFAULT_RUN_HISTORY_PAGE_SIZE)
        .clamp(1, MAX_RUN_HISTORY_PAGE_SIZE);
    let mut entries = state
        .metadata
        .runs()
        .repository_run_history_page(RunHistoryPageQuery {
            repository_id: &repo.record.id,
            workflow_path: workflow_path.as_deref(),
            git_oid: git_oid.as_deref(),
            after: after.as_ref(),
            limit: (limit + 1) as u64,
        })
        .await?;
    let has_more = entries.len() > limit;
    entries.truncate(limit);
    let next_cursor = has_more.then(|| {
        let last = entries
            .last()
            .expect("a run history page with more results is non-empty");
        encode_history_cursor(last.creation_sequence, workflow, git_oid.as_deref())
    });
    let runs = entries
        .iter()
        .map(|entry| repository_run_summary(&entry.run, &entry.jobs))
        .collect::<Vec<_>>();
    Ok(Json(RepositoryRunHistoryPageResponse { runs, next_cursor }))
}

async fn current_workflows(
    state: &AppState,
    repository_id: &str,
) -> Result<Vec<WorkflowRevision>, ApiError> {
    let snapshot = state
        .metadata
        .repositories()
        .current_repository_workflow_catalog(repository_id)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| {
            ApiError::internal_message("repository disappeared while loading current workflows")
        })?;
    let Some(head) = snapshot.git_head.as_ref() else {
        return if snapshot.catalog.is_none() {
            Ok(Vec::new())
        } else {
            Err(ApiError::internal_message(
                "repository workflow catalog exists without an accepted Git head",
            ))
        };
    };
    let catalog = snapshot.catalog.ok_or_else(|| {
        ApiError::internal_message("repository workflow catalog is missing for current main")
    })?;
    catalog
        .verify_source(&snapshot.repository_id, &head.head_oid, head.change_version)
        .map_err(ApiError::internal)?;
    scope_run_config::parse_repository_workflow_catalog(&catalog).map_err(ApiError::bad_request)
}

fn parse_history_cursor(
    value: &str,
    workflow: Option<&str>,
    git_oid: Option<&str>,
) -> Result<RunHistoryCursor, ApiError> {
    let mut parts = value.splitn(4, ':');
    if parts.next() != Some("v3") {
        return Err(ApiError::bad_request("invalid run history cursor"));
    }
    let creation_sequence = parts
        .next()
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| ApiError::bad_request("invalid run history cursor"))?;
    let cursor_oid = parts
        .next()
        .ok_or_else(|| ApiError::bad_request("invalid run history cursor"))?;
    let cursor_workflow = parts
        .next()
        .ok_or_else(|| ApiError::bad_request("invalid run history cursor"))?;
    if cursor_workflow != workflow.unwrap_or("*") || cursor_oid != git_oid.unwrap_or("*") {
        return Err(ApiError::bad_request(
            "run history cursor does not match the filters",
        ));
    }
    Ok(RunHistoryCursor { creation_sequence })
}

fn encode_history_cursor(
    creation_sequence: u64,
    workflow: Option<&str>,
    git_oid: Option<&str>,
) -> String {
    format!(
        "v3:{creation_sequence}:{}:{}",
        git_oid.unwrap_or("*"),
        workflow.unwrap_or("*")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_cursor_is_bound_to_the_filters() {
        let oid = "a".repeat(40);
        let other_oid = "b".repeat(40);
        let encoded = encode_history_cursor(42, Some("checks"), Some(&oid));
        assert_eq!(
            parse_history_cursor(&encoded, Some("checks"), Some(&oid)).unwrap(),
            RunHistoryCursor {
                creation_sequence: 42
            }
        );
        assert!(parse_history_cursor(&encoded, None, Some(&oid)).is_err());
        assert!(parse_history_cursor(&encoded, Some("checks"), None).is_err());
        assert!(parse_history_cursor(&encoded, Some("checks"), Some(&other_oid)).is_err());
        assert!(parse_history_cursor("v3:42", Some("checks"), Some(&oid)).is_err());
        let unfiltered = encode_history_cursor(42, None, None);
        assert!(parse_history_cursor(&unfiltered, None, None).is_ok());
        assert!(parse_history_cursor(&unfiltered, None, Some(&oid)).is_err());
    }
}
