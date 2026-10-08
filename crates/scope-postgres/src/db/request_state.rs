use super::{RequestInviteeRead, RequestStore, begin_metadata_read_snapshot};
use crate::error::PostgresError;
use scope_domain::{
    account::UserAccount,
    github_connection::GitHubConnection,
    repository::{access::RepositoryAccessContext, repo_id},
    requests::{
        GitHubPush, Request, RequestAutoMergeIntent, RequestCheckEvaluation, RequestCheckResults,
        RequestRevision, RequestViewer, request_policy,
    },
};
use std::collections::{BTreeMap, HashMap};

pub struct RequestStateSnapshot {
    pub repository: RepositoryAccessContext,
    pub request: Request,
    pub is_invitee: bool,
    pub viewer_user_id: Option<String>,
    pub invitees: Vec<RequestInviteeRead>,
    pub evaluation: Option<RequestCheckEvaluation>,
    pub results: RequestCheckResults,
    pub revision: Option<RequestRevision>,
    pub auto_merge: Option<RequestAutoMergeIntent>,
    pub users: BTreeMap<String, UserAccount>,
    pub github_connection: Option<GitHubConnection>,
    pub github_push: Option<GitHubPush>,
    pub github_runs: HashMap<u64, u64>,
    pub current_main_oid: Option<String>,
}

impl RequestStore {
    pub async fn request_state_snapshot(
        &self,
        owner: &str,
        name: &str,
        request_id: &str,
        viewer_user_id: Option<&str>,
    ) -> Result<RequestStateSnapshot, PostgresError> {
        let tx = begin_metadata_read_snapshot(self.db.as_ref()).await?;
        let repository =
            super::repository_access::repository_access(&tx, &repo_id(owner, name), viewer_user_id)
                .await?
                .ok_or_else(|| PostgresError::not_found("repo not found"))?;
        let projection = super::projection_read_models::live_projection_read_model(
            &tx,
            &repository.record.id,
            repository.record.content_version,
            &repository.access.view,
        )
        .await?;
        if !repository.can_read(
            projection
                .as_ref()
                .is_some_and(|projection| projection.visible_files),
        ) {
            return Err(PostgresError::not_found("repo not found"));
        }
        let request = super::request_rows::request_by_id(&tx, request_id)
            .await?
            .filter(|request| request.repo_id == repository.record.id)
            .ok_or_else(|| PostgresError::not_found("request not found"))?;
        let is_invitee = match viewer_user_id {
            Some(user_id) => {
                super::request_invitees::request_is_invitee(&tx, request_id, user_id).await?
            }
            None => false,
        };
        let viewer = RequestViewer::new(repository.access.clone(), viewer_user_id, is_invitee);
        let policy = request_policy(&request, viewer, &repository.views);
        if !policy.exact_visible {
            return Err(PostgresError::not_found("request not found"));
        }
        let invitees = if repository.views.anyone() == Some(&request.view) {
            super::request_invitees::request_invitee_reads(&tx, request_id).await?
        } else {
            Vec::new()
        };
        let evaluation =
            super::request_checks::evaluation_for_head(&tx, request_id, &request.head_oid).await?;
        let results = super::request_checks::request_check_results(
            &tx,
            &request.repo_id,
            evaluation.as_ref(),
        )
        .await?;
        let revision =
            super::request_revision_rows::latest_revision_for_request(&tx, request_id).await?;
        let auto_merge = super::request_auto_merge::latest_intent(&tx, request_id).await?;
        let users = super::auth::load_users_by_ids(
            &tx,
            auto_merge
                .iter()
                .map(|intent| intent.actor_user_id.clone())
                .chain(viewer_user_id.map(str::to_string)),
        )
        .await?;
        let github_connection =
            super::github_connections::repository_github_connection(&tx, &request.repo_id).await?;
        let github_push = super::github_pushes::latest_github_push(&tx, request_id).await?;
        let suites = evaluation
            .iter()
            .flat_map(|evaluation| {
                evaluation.checks.iter().filter_map(|check| match check {
                    scope_domain::requests::RequestCheck::GitHub { name } => {
                        results
                            .github
                            .latest(&evaluation.tested_oid, name)?
                            .check_suite_id
                    }
                    scope_domain::requests::RequestCheck::Native(_) => None,
                })
            })
            .collect::<Vec<_>>();
        let github_runs = match github_connection.as_ref() {
            Some(connection) => super::github_workflow_runs::github_workflow_runs_for_check_suites(
                &tx,
                &request.repo_id,
                connection.github_repository_id,
                &suites,
            )
            .await?
            .into_iter()
            .collect(),
            None => HashMap::new(),
        };
        let current_main_oid = if &repository.access.view == repository.views.full() {
            super::request_check_commits::canonical_main_oid(&tx, &request.repo_id)
                .await?
                .or_else(|| projection.and_then(|projection| projection.head_oid))
        } else {
            projection.and_then(|projection| projection.head_oid)
        };
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(RequestStateSnapshot {
            repository,
            request,
            is_invitee,
            viewer_user_id: viewer_user_id.map(str::to_string),
            invitees,
            evaluation,
            results,
            revision,
            auto_merge,
            users,
            github_connection,
            github_push,
            github_runs,
            current_main_oid,
        })
    }
}
