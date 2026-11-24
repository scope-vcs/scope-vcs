use super::{
    GitHubWorkflowRunReference, RequestInviteeRead, RequestStore, begin_metadata_read_snapshot,
    entities, request_ratings::author_standings,
};
use crate::error::PostgresError;
use scope_domain::{
    account::UserAccount,
    github::connection::GitHubConnection,
    repository::{
        RepoLifecycleState,
        access::{RepositoryAccess, RepositoryAccessContext},
        repo_id,
    },
    requests::{
        AuthorRecord, AuthoredRequest, GitHubPullRequest, GitHubPush, Request,
        RequestAutoMergeIntent, RequestCheckEvaluation, RequestCheckResults, RequestRevision,
        RequestViewer, author_record_subject, request_policy,
    },
};
use sea_orm::DatabaseTransaction;
use tracing::Instrument as _;

mod rows;
use std::collections::{BTreeMap, HashMap};

pub struct RequestStateSnapshot {
    pub stack_navigation: scope_domain::requests::RequestStackNavigation,
    pub children: super::RequestChildrenPage,
    pub repository: RepositoryAccessContext,
    pub request: Request,
    pub author_record: Option<AuthorRecord>,
    pub is_invitee: bool,
    pub viewer_user_id: Option<String>,
    pub invitees: Vec<RequestInviteeRead>,
    pub evaluation: Option<RequestCheckEvaluation>,
    pub results: RequestCheckResults,
    pub revision: Option<RequestRevision>,
    pub auto_merge: Option<RequestAutoMergeIntent>,
    pub auto_merge_actor_access: Option<RepositoryAccess>,
    pub users: BTreeMap<String, UserAccount>,
    pub claimer_user_id: Option<String>,
    pub asker: Option<scope_domain::github::account::GitHubAccount>,
    pub github_connection: Option<GitHubConnection>,
    pub github_push: Option<GitHubPush>,
    pub github_pull_request: Option<GitHubPullRequest>,
    pub bot_reviews: scope_domain::requests::RequestBotReviews,
    pub github_runs: HashMap<u64, GitHubWorkflowRunReference>,
    pub current_main_oid: Option<String>,
    pub view_mains: scope_domain::requests::RequestViewMains,
}

impl RequestStateSnapshot {
    pub fn checks_outcome(&self) -> scope_domain::requests::RequestChecksOutcome {
        scope_domain::requests::request_checks_outcome(
            &self.request.id,
            &self.request.head_oid,
            self.evaluation.as_ref(),
            &self.results,
        )
    }

    pub fn reviews_outcome(&self) -> scope_domain::requests::RequestReviewsOutcome {
        scope_domain::requests::request_reviews_outcome(
            &self.request.view,
            &self.request.head_oid,
            &self.repository.views,
            &self.bot_reviews,
        )
    }
}

impl RequestStore {
    pub async fn request_state_snapshot(
        &self,
        owner: &str,
        name: &str,
        request_id: &str,
        viewer_user_id: Option<&str>,
    ) -> Result<RequestStateSnapshot, PostgresError> {
        self.request_state_snapshot_with_children(
            owner,
            name,
            request_id,
            viewer_user_id,
            None,
            scope_domain::requests::REQUEST_LIST_DEFAULT_PAGE_SIZE,
        )
        .await
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "request_state_snapshot"))]
    pub async fn request_state_snapshot_with_children(
        &self,
        owner: &str,
        name: &str,
        request_id: &str,
        viewer_user_id: Option<&str>,
        children_after_id: Option<&str>,
        children_limit: usize,
    ) -> Result<RequestStateSnapshot, PostgresError> {
        for _ in 0..3 {
            let tx = begin_metadata_read_snapshot(self.db.as_ref()).await?;
            let read =
                super::repository_access::load_access(&tx, &repo_id(owner, name), viewer_user_id)
                    .await?
                    .ok_or_else(|| PostgresError::not_found("repo not found"))?;
            let projection = read.projection(&read.context.access.view).cloned();
            let view_mains = read.request_view_mains();
            let repository = read.context;
            if projection.is_none()
                && repository.record.lifecycle_state == RepoLifecycleState::Ready
                && repository.can_read_view(&repository.access.view)
            {
                tx.commit().await?;
                super::history_reads::ensure_live_projection_read_models(
                    self.db.as_ref(),
                    &repository.incarnation(),
                    &repository.access.view,
                )
                .await?;
                continue;
            }
            return read_request_state(
                tx,
                repository,
                (projection, view_mains),
                request_id,
                viewer_user_id,
                children_after_id,
                children_limit,
            )
            .await;
        }
        Err(PostgresError::conflict(
            "repository kept changing while reading the request; retry",
        ))
    }
}

async fn read_request_state(
    tx: DatabaseTransaction,
    repository: RepositoryAccessContext,
    (projection, view_mains): (
        Option<entities::projection_read_model::Model>,
        scope_domain::requests::RequestViewMains,
    ),
    request_id: &str,
    viewer_user_id: Option<&str>,
    children_after_id: Option<&str>,
    children_limit: usize,
) -> Result<RequestStateSnapshot, PostgresError> {
    if !repository.can_read(
        projection
            .as_ref()
            .is_some_and(|projection| projection.visible_files),
    ) {
        return Err(PostgresError::not_found("repo not found"));
    }
    let row = rows::load(&tx, &repository.record.id, request_id, viewer_user_id)
        .await?
        .ok_or_else(|| PostgresError::not_found("request not found"))?;
    let request = row.request()?;
    let is_invitee = row.is_invitee()?;
    let viewer = RequestViewer::new(repository.access.clone(), viewer_user_id, is_invitee);
    let policy = request_policy(&request, viewer, &repository.views);
    if !policy.exact_visible {
        return Err(PostgresError::not_found("request not found"));
    }
    let invitees = if policy.invitees_visible {
        super::request_invitees::request_invitee_reads(&tx, request_id)
            .instrument(tracing::info_span!(
                "request_invitee_reads",
                otel.kind = "client",
                db.system.name = "postgresql",
                db.operation.name = "request_invitee_reads"
            ))
            .await?
    } else {
        Vec::new()
    };
    let author_record = match author_record_subject(
        AuthoredRequest {
            author_user_id: request.author_user_id.as_deref(),
            state: request.state(),
        },
        &repository.access,
        &repository.views,
    ) {
        Some(subject) => {
            let standing =
                author_standings(&tx, &request.repo_id, vec![subject.user_id.to_owned()])
                    .await?
                    .remove(subject.user_id)
                    .ok_or_else(|| PostgresError::internal_message("author standing is missing"))?;
            let author = standing.access(&repository.record, subject.user_id);
            subject.record(&author, standing.totals)?
        }
        None => None,
    };
    let reads_change = policy.change_visible;
    let request = if reads_change {
        request
    } else {
        request.with_change_hidden()
    };
    let evaluation = row.evaluation()?.filter(|_| reads_change);
    let results =
        super::request_checks::request_check_results(&tx, &request.repo_id, evaluation.as_ref())
            .instrument(tracing::info_span!(
                "request_check_results",
                otel.kind = "client",
                db.system.name = "postgresql",
                db.operation.name = "request_check_results"
            ))
            .await?;
    let revision = row.revision()?.filter(|_| reads_change);
    let auto_merge = row.auto_merge()?.filter(|_| reads_change);
    let auto_merge_actor_access = row.auto_merge_actor_access(&repository, auto_merge.as_ref())?;
    let users = row.users()?;
    let claimer_user_id = row.claimer_user_id()?;
    let asker = super::request_asks::asker(&tx, &request).await?;
    let github_connection =
        super::github::connections::repository_github_connection(&tx, &request.repo_id)
            .instrument(tracing::info_span!(
                "repository_github_connection",
                otel.kind = "client",
                db.system.name = "postgresql",
                db.operation.name = "repository_github_connection"
            ))
            .await?;
    let github_push = super::github::pushes::latest_github_push(&tx, request_id)
        .instrument(tracing::info_span!(
            "latest_github_push",
            otel.kind = "client",
            db.system.name = "postgresql",
            db.operation.name = "latest_github_push"
        ))
        .await?
        .filter(|_| reads_change);
    let github_pull_request = super::github::pull_requests::github_pull_request(&tx, request_id)
        .await?
        .filter(|_| reads_change);
    let bot_reviews = super::github::review_bot_statuses::request_bot_reviews(
        &tx,
        &request.repo_id,
        std::slice::from_ref(&request.id),
    )
    .instrument(tracing::info_span!(
        "request_bot_reviews",
        otel.kind = "client",
        db.system.name = "postgresql",
        db.operation.name = "request_bot_reviews"
    ))
    .await?
    .remove(&request.id)
    .filter(|_| reads_change)
    .unwrap_or_default();
    let suites = evaluation
        .iter()
        .flat_map(|evaluation| {
            evaluation.checks.iter().filter_map(|check| {
                results
                    .github
                    .latest(&evaluation.tested_oid, &check.name)?
                    .check_suite_id
            })
        })
        .collect::<Vec<_>>();
    let github_runs = match github_connection.as_ref() {
        Some(connection) => super::github::workflow_runs::github_workflow_runs_for_check_suites(
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
        row.canonical_main_oid()?
            .or_else(|| projection.and_then(|projection| projection.head_oid))
    } else {
        projection.and_then(|projection| projection.head_oid)
    };
    let (stack_navigation, children) = if reads_change {
        super::request_navigation::navigation(
            &tx,
            &repository,
            viewer_user_id,
            &request,
            children_after_id,
            children_limit,
        )
        .await?
    } else {
        Default::default()
    };
    tx.commit()
        .instrument(tracing::info_span!(
            "commit_read_snapshot",
            otel.kind = "client",
            db.system.name = "postgresql",
            db.operation.name = "commit_read_snapshot"
        ))
        .await?;
    Ok(RequestStateSnapshot {
        stack_navigation,
        children,
        repository,
        request,
        author_record,
        is_invitee,
        viewer_user_id: viewer_user_id.map(str::to_string),
        invitees,
        evaluation,
        results,
        revision,
        auto_merge,
        auto_merge_actor_access,
        users,
        claimer_user_id,
        asker,
        github_connection,
        github_push,
        github_pull_request,
        bot_reviews,
        github_runs,
        current_main_oid,
        view_mains,
    })
}
