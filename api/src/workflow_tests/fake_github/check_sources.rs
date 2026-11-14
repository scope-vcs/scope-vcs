use super::{FakeGitHub, InstallationState};
use axum::{
    Json, Router,
    extract::{Path as AxumPath, Query as AxumQuery, State as AxumState},
    http::{HeaderMap, StatusCode, header::AUTHORIZATION},
    response::IntoResponse,
    routing::get,
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

pub(in crate::workflow_tests) struct FakeCheckSources {
    pub(in crate::workflow_tests) permissions: Mutex<serde_json::Value>,
    pub(in crate::workflow_tests) protected_contexts: Mutex<Vec<String>>,
    pub(in crate::workflow_tests) ruleset_contexts: Mutex<Vec<String>>,
    pub(in crate::workflow_tests) workflows: Mutex<Vec<serde_json::Value>>,
}

impl Default for FakeCheckSources {
    fn default() -> Self {
        Self {
            permissions: Mutex::new(serde_json::json!({
                "actions": "read", "checks": "read", "contents": "write", "issues": "read",
                "metadata": "read", "pull_requests": "write", "workflows": "write",
            })),
            protected_contexts: Mutex::default(),
            ruleset_contexts: Mutex::default(),
            workflows: Mutex::default(),
        }
    }
}

fn first_page(query: &HashMap<String, String>) -> bool {
    query.get("page").is_none_or(|page| page == "1")
}

fn installation_id(headers: &HeaderMap) -> Option<u64> {
    headers
        .get(AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer installation-token-")?
        .parse()
        .ok()
}

pub(super) fn routes() -> Router<Arc<FakeGitHub>> {
    Router::new()
        .route(
            "/repositories/{id}",
            get(
                |AxumState(fake): AxumState<Arc<FakeGitHub>>,
                 AxumPath(id): AxumPath<u64>,
                 headers: HeaderMap| async move {
                    let installation = installation_id(&headers);
                    let suspended = installation.is_some_and(|installation| {
                        fake.installation_states.lock().unwrap().get(&installation)
                            == Some(&InstallationState::Suspended)
                    });
                    if suspended {
                        return StatusCode::FORBIDDEN.into_response();
                    }
                    fake.installation_repositories
                        .lock()
                        .unwrap()
                        .iter()
                        .find(|(listed_in, repository)| {
                            Some(*listed_in) == installation && repository["id"] == id
                        })
                        .map(|(_, repository)| Json(repository.clone()).into_response())
                        .unwrap_or_else(|| StatusCode::NOT_FOUND.into_response())
                },
            ),
        )
        .route(
            "/repos/{owner}/{name}/branches/{branch}",
            get(
                |AxumState(fake): AxumState<Arc<FakeGitHub>>,
                 AxumPath((owner, name, branch)): AxumPath<(String, String, String)>| async move {
                    let Some(sha) = fake.branch_head_of(&format!("{owner}/{name}"), &branch)
                    else {
                        return StatusCode::NOT_FOUND.into_response();
                    };
                    let contexts = fake.check_sources.protected_contexts.lock().unwrap().clone();
                    Json(serde_json::json!({
                        "name": branch,
                        "commit": { "sha": sha },
                        "protected": !contexts.is_empty(),
                        "protection": {
                            "enabled": !contexts.is_empty(),
                            "required_status_checks": { "contexts": contexts },
                        },
                    }))
                    .into_response()
                },
            ),
        )
        .route(
            "/repos/{owner}/{name}/rules/branches/{branch}",
            get(
                |AxumState(fake): AxumState<Arc<FakeGitHub>>,
                 AxumQuery(query): AxumQuery<HashMap<String, String>>| async move {
                    if !first_page(&query) {
                        return Json(serde_json::json!([]));
                    }
                    let contexts = fake.check_sources.ruleset_contexts.lock().unwrap().clone();
                    Json(serde_json::json!([
                        { "type": "deletion", "ruleset_id": 1 },
                        {
                            "type": "required_status_checks",
                            "ruleset_id": 2,
                            "parameters": {
                                "required_status_checks": contexts
                                    .iter()
                                    .map(|context| serde_json::json!({ "context": context }))
                                    .collect::<Vec<_>>(),
                            },
                        },
                    ]))
                },
            ),
        )
        .route(
            "/repos/{owner}/{name}/actions/workflows",
            get(
                |AxumState(fake): AxumState<Arc<FakeGitHub>>,
                 AxumQuery(query): AxumQuery<HashMap<String, String>>| async move {
                    let workflows = if first_page(&query) {
                        fake.check_sources.workflows.lock().unwrap().clone()
                    } else {
                        Vec::new()
                    };
                    Json(serde_json::json!({
                        "total_count": workflows.len(),
                        "workflows": workflows,
                    }))
                },
            ),
        )
}
