use super::{GitHubApp, GitHubRepository, client::unavailable};
use crate::error::ApiError;
use reqwest::{Method, StatusCode};
use scope_domain::github_check_catalog::GitHubWorkflowState;
use serde::Deserialize;

const BRANCH_RULES_ROUTE: &str = "/repos/{owner}/{repo}/rules/branches/{branch}";

#[derive(Deserialize)]
struct Branch {
    commit: BranchCommit,
    protection: Option<BranchProtection>,
}

#[derive(Deserialize)]
struct BranchCommit {
    sha: String,
}

#[derive(Deserialize)]
struct BranchProtection {
    required_status_checks: Option<RequiredStatusChecks>,
}

#[derive(Deserialize)]
struct RequiredStatusChecks {
    #[serde(default)]
    contexts: Vec<String>,
}

#[derive(Deserialize)]
struct BranchRule {
    #[serde(rename = "type")]
    kind: String,
    parameters: Option<BranchRuleParameters>,
}

#[derive(Deserialize)]
struct BranchRuleParameters {
    #[serde(default)]
    required_status_checks: Vec<RequiredStatusCheck>,
}

#[derive(Deserialize)]
struct RequiredStatusCheck {
    context: String,
}

#[derive(Deserialize)]
struct WorkflowsPage {
    workflows: Vec<Workflow>,
}

#[derive(Deserialize)]
struct Workflow {
    path: String,
    state: String,
}

pub(crate) struct GitHubBranchChecks {
    pub(crate) head_oid: String,
    pub(crate) required: Vec<String>,
}

impl GitHubApp {
    pub(crate) async fn repository(
        &self,
        installation_id: u64,
        github_repository_id: u64,
    ) -> Result<Option<GitHubRepository>, ApiError> {
        self.installation_get(
            installation_id,
            &format!("/repositories/{github_repository_id}"),
            "/repositories/{repository_id}",
        )
        .await
    }

    pub(crate) async fn branch_checks(
        &self,
        installation_id: u64,
        full_name: &str,
        branch: &str,
    ) -> Result<Option<GitHubBranchChecks>, ApiError> {
        let protected = self
            .installation_get::<Branch>(
                installation_id,
                &format!("/repos/{full_name}/branches/{branch}"),
                "/repos/{owner}/{repo}/branches/{branch}",
            )
            .await?;
        let Some(protected) = protected else {
            return Ok(None);
        };
        let rules = self
            .branch_rules(installation_id, full_name, branch)
            .await?;
        let required = protected
            .protection
            .and_then(|protection| protection.required_status_checks)
            .map(|checks| checks.contexts)
            .unwrap_or_default()
            .into_iter()
            .chain(
                rules
                    .into_iter()
                    .filter(|rule| rule.kind == "required_status_checks")
                    .filter_map(|rule| rule.parameters)
                    .flat_map(|parameters| parameters.required_status_checks)
                    .map(|check| check.context),
            )
            .collect();
        Ok(Some(GitHubBranchChecks {
            head_oid: protected.commit.sha,
            required,
        }))
    }

    async fn branch_rules(
        &self,
        installation_id: u64,
        full_name: &str,
        branch: &str,
    ) -> Result<Vec<BranchRule>, ApiError> {
        let Some(token) = self.installation_token(installation_id).await? else {
            return Ok(Vec::new());
        };
        let request = self
            .request(
                Method::GET,
                &format!("/repos/{full_name}/rules/branches/{branch}?per_page=100"),
            )
            .bearer_auth(token);
        let response =
            scope_service_runtime::outbound_http::send_traced(request, BRANCH_RULES_ROUTE)
                .await
                .map_err(|error| {
                    unavailable(format!("GitHub was unreachable: {}", error.without_url()))
                })?;
        match response.status() {
            StatusCode::FORBIDDEN | StatusCode::NOT_FOUND => Ok(Vec::new()),
            status if !status.is_success() => Err(unavailable(format!(
                "GitHub answered {status} for the rules of {full_name}'s {branch}"
            ))),
            _ => response
                .json()
                .await
                .map_err(|error| unavailable(format!("GitHub answered unreadably: {error}"))),
        }
    }

    pub(crate) async fn workflow_states(
        &self,
        installation_id: u64,
        full_name: &str,
    ) -> Result<Vec<GitHubWorkflowState>, ApiError> {
        Ok(self
            .installation_pages::<WorkflowsPage, _>(
                installation_id,
                &format!("/repos/{full_name}/actions/workflows"),
                "/repos/{owner}/{repo}/actions/workflows",
                |page| page.workflows,
            )
            .await?
            .unwrap_or_default()
            .into_iter()
            .map(|workflow| GitHubWorkflowState {
                active: workflow.state == "active",
                path: workflow.path,
            })
            .collect())
    }
}
