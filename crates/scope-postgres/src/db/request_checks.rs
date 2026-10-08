use super::{
    RequestStore, entities,
    github_connections::repository_github_connection,
    github_pushes::queue_github_push,
    native_runs::lock_native_runs_availability,
    request_access::{ensure_user_exists, lock_request_repository},
    runs::{enqueue_run_in_transaction, save_workflow_revision},
};
use crate::error::PostgresError;
use scope_domain::{
    requests::{
        GitHubBranch, GitHubCheckResults, GitHubPushDestination, Request, RequestCheckEvaluation,
        RequestCheckPlan, RequestCheckResults, RequestCheckReviewer, RequestRevision,
        ensure_approving_reviewed_head, stop_request_auto_merge_for_check_evaluation,
    },
    runs::{
        run::Run,
        workflow::{identity::WorkflowIdentity, revision::WorkflowRevision},
    },
    views::ViewId,
};
use sea_orm::{
    ColumnTrait, Condition, ConnectionTrait, DatabaseTransaction, EntityTrait, IntoActiveModel,
    QueryFilter, TransactionTrait, sea_query::OnConflict,
};

#[derive(Clone, Debug)]
pub struct RecordRequestChecksCommand {
    pub repository_incarnation: scope_domain::repository::RepositoryIncarnation,
    pub expected_canonical_main_oid: Option<String>,
    pub evaluation: RequestCheckEvaluation,
    pub revisions: Vec<WorkflowRevision>,
    pub runs: Vec<Run>,
    pub push_to_github: bool,
}

#[derive(Clone, Debug)]
pub struct ApproveRequestChecksCommand {
    pub request_id: String,
    pub actor_user_id: String,
    pub reviewed_head_oid: String,
    pub now_unix: u64,
}

#[derive(Clone, Debug)]
pub struct RequestChecksMutation {
    pub evaluation: RequestCheckEvaluation,
    pub created_runs: Vec<Run>,
    pub queued_github_push: bool,
}

impl RequestStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "record_request_checks"))]
    pub async fn record_request_checks(
        &self,
        command: RecordRequestChecksCommand,
    ) -> Result<RequestChecksMutation, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        super::locks::acquire_shared_repository_lock(
            &tx,
            command.repository_incarnation.repository_id(),
        )
        .await?;
        super::acquire_aggregate_lock(&tx, "request", &command.evaluation.request_id).await?;
        super::run_retention::lock_run_evidence_retention(&tx).await?;
        let active_auto_merge = super::request_auto_merge::lock_active_intent_for_request(
            &tx,
            &command.evaluation.request_id,
        )
        .await?;
        let request = super::request_rows::request_by_id(&tx, &command.evaluation.request_id)
            .await?
            .ok_or_else(|| PostgresError::not_found("request not found"))?;
        let repo = super::repository_access::load_repo_record(&tx, &request.repo_id)
            .await?
            .ok_or_else(|| PostgresError::not_found("repo not found"))?;
        if repo.incarnation() != command.repository_incarnation
            || request.head_oid != command.evaluation.head_oid
        {
            return Err(PostgresError::conflict(
                "request changed while evaluating checks",
            ));
        }
        let canonical_main_oid =
            super::request_check_commits::canonical_main_oid(&tx, &request.repo_id).await?;
        if canonical_main_oid != command.expected_canonical_main_oid {
            return Err(PostgresError::conflict(
                "main changed while evaluating checks",
            ));
        }
        if let Some(base) = &command.evaluation.check_commit_base
            && canonical_main_oid.as_deref() != Some(&base.canonical_main_oid)
        {
            return Err(PostgresError::conflict(
                "main changed while evaluating checks",
            ));
        }
        if request.is_terminal() {
            return Err(PostgresError::conflict(
                "request can no longer merge, so its checks are not evaluated",
            ));
        }
        if let Some(evaluation) = evaluation_for_head(
            &tx,
            &command.evaluation.request_id,
            &command.evaluation.head_oid,
        )
        .await?
        {
            return Ok(RequestChecksMutation {
                evaluation,
                created_runs: Vec::new(),
                queued_github_push: false,
            });
        }
        if command.evaluation.uses_native_runs() {
            lock_native_runs_availability(&tx, &request.repo_id)
                .await?
                .require()?;
        }
        let created_runs = start_runs(&tx, &command.revisions, command.runs).await?;
        for revision in &command.revisions {
            save_workflow_revision(&tx, revision, command.evaluation.updated_at_unix).await?;
        }
        save_evaluation(&tx, &command.evaluation).await?;
        let queued_github_push = command.push_to_github
            && queue_tested_commit_push(
                &tx,
                &request,
                &command.evaluation.tested_oid,
                command.evaluation.updated_at_unix,
            )
            .await?;
        stop_auto_merge_for_evaluation(&tx, active_auto_merge, &request, &command.evaluation)
            .await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(RequestChecksMutation {
            evaluation: command.evaluation,
            created_runs,
            queued_github_push,
        })
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "approve_request_checks"))]
    pub async fn approve_request_checks(
        &self,
        command: ApproveRequestChecksCommand,
    ) -> Result<RequestChecksMutation, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let (repo, request) =
            lock_request_repository(&tx, &command.request_id, &command.actor_user_id).await?;
        super::run_retention::lock_run_evidence_retention(&tx).await?;
        ensure_user_exists(&tx, &command.actor_user_id).await?;
        let approver =
            RequestCheckReviewer::for_actor(&command.actor_user_id, &repo.access, &repo.views)
                .ok_or_else(|| {
                    PostgresError::permission_denied(RequestCheckReviewer::refusal(
                        &repo.access,
                        &repo.views,
                    ))
                })?;
        ensure_approving_reviewed_head(&request, &command.reviewed_head_oid)?;
        let evaluation = evaluation_for_head(&tx, &request.id, &request.head_oid)
            .await?
            .ok_or_else(|| PostgresError::not_found("request head has no recorded checks"))?;
        evaluation.ensure_awaiting_approval()?;
        let mut revisions = Vec::new();
        for check in evaluation.native_checks() {
            let identity = WorkflowIdentity::new(
                &request.repo_id,
                scope_domain::runs::workflow::identity::WorkflowPath::parse(
                    check.workflow_path.clone(),
                )
                .map_err(PostgresError::invalid_input)?,
            )
            .map_err(PostgresError::invalid_input)?;
            let revision = entities::workflow_revision::Entity::find_by_id(
                check.workflow_revision_digest.clone(),
            )
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::internal_message("recorded check revision is missing"))?
            .try_into_domain(identity)?;
            revisions.push(revision);
        }
        let RequestCheckPlan {
            evaluation,
            runs,
            push_to_github,
        } = RequestCheckPlan::approve(
            &request,
            evaluation,
            &revisions,
            approver,
            command.now_unix,
        )?;
        let created_runs = start_runs(&tx, &revisions, runs).await?;
        save_evaluation(&tx, &evaluation).await?;
        let queued_github_push = push_to_github
            && queue_tested_commit_push(&tx, &request, &evaluation.tested_oid, command.now_unix)
                .await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(RequestChecksMutation {
            evaluation,
            created_runs,
            queued_github_push,
        })
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "latest_request_revision"))]
    pub async fn latest_request_revision(
        &self,
        request_id: &str,
    ) -> Result<Option<RequestRevision>, PostgresError> {
        super::request_revision_rows::latest_revision_for_request(self.db.as_ref(), request_id)
            .await
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "request_revision_with_head"))]
    pub async fn request_revision_with_head(
        &self,
        request_id: &str,
        head_oid: &str,
    ) -> Result<Option<RequestRevision>, PostgresError> {
        super::request_revision_rows::latest_revision_with_head(
            self.db.as_ref(),
            request_id,
            head_oid,
        )
        .await
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "request_check_evaluation"))]
    pub async fn request_check_evaluation(
        &self,
        request_id: &str,
        head_oid: &str,
    ) -> Result<Option<RequestCheckEvaluation>, PostgresError> {
        evaluation_for_head(self.db.as_ref(), request_id, head_oid).await
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "request_check_evaluation_testing"))]
    pub async fn request_check_evaluation_testing(
        &self,
        request_id: &str,
        tested_oid: &str,
    ) -> Result<Option<RequestCheckEvaluation>, PostgresError> {
        entities::request_check_evaluation::Entity::find()
            .filter(entities::request_check_evaluation::Column::RequestId.eq(request_id))
            .filter(entities::request_check_evaluation::Column::TestedOid.eq(tested_oid))
            .one(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?
            .map(entities::request_check_evaluation::Model::try_into_domain)
            .transpose()
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "request_check_results"))]
    pub async fn request_check_results(
        &self,
        repo_id: &str,
        evaluations: &[RequestCheckEvaluation],
    ) -> Result<RequestCheckResults, PostgresError> {
        request_check_results(self.db.as_ref(), repo_id, evaluations).await
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "request_check_evaluations"))]
    pub async fn request_check_evaluations(
        &self,
        heads: &[(String, String)],
    ) -> Result<Vec<RequestCheckEvaluation>, PostgresError> {
        if heads.is_empty() {
            return Ok(Vec::new());
        }
        entities::request_check_evaluation::Entity::find()
            .filter(head_pairs_condition(heads))
            .all(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?
            .into_iter()
            .map(entities::request_check_evaluation::Model::try_into_domain)
            .collect()
    }
}

fn head_pairs_condition(heads: &[(String, String)]) -> Condition {
    heads
        .iter()
        .fold(Condition::any(), |condition, (request_id, head_oid)| {
            condition.add(
                Condition::all()
                    .add(
                        entities::request_check_evaluation::Column::RequestId
                            .eq(request_id.clone()),
                    )
                    .add(entities::request_check_evaluation::Column::HeadOid.eq(head_oid.clone())),
            )
        })
}

pub(super) async fn request_check_results<'a, C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
    evaluations: impl IntoIterator<Item = &'a RequestCheckEvaluation>,
) -> Result<RequestCheckResults, PostgresError> {
    let mut run_ids = Vec::new();
    let mut tested_oids = Vec::new();
    let mut github_request_ids = Vec::new();
    let mut check_commit_request_ids = Vec::new();
    for evaluation in evaluations {
        run_ids.extend(evaluation.run_ids().map(str::to_string));
        if evaluation.asks_github() {
            tested_oids.push(evaluation.tested_oid.clone());
            if evaluation.tests_check_commit() {
                check_commit_request_ids.push(evaluation.request_id.clone());
            } else {
                github_request_ids.push(evaluation.request_id.clone());
            }
        }
    }
    let native_runs = if run_ids.is_empty() {
        Vec::new()
    } else {
        entities::run::Entity::find()
            .filter(entities::run::Column::Id.is_in(run_ids))
            .all(conn)
            .await
            .map_err(PostgresError::internal)?
            .into_iter()
            .map(|row| Ok((row.id.clone(), row.try_into_domain()?.state)))
            .collect::<Result<_, PostgresError>>()?
    };
    let connection = if tested_oids.is_empty() {
        None
    } else {
        repository_github_connection(conn, repo_id)
            .await?
            .filter(|connection| connection.is_connected())
    };
    let withheld_from_github = match &connection {
        Some(connection) if !connection.may_receive_private_requests() => {
            let views = super::projection_read_models::repository_views(conn, repo_id).await?;
            let mut withheld =
                requests_outside_anyone_view(conn, &github_request_ids, views.anyone()).await?;
            withheld.extend(check_commit_request_ids.iter().cloned());
            withheld
        }
        _ => Vec::new(),
    };
    let github = if tested_oids.is_empty() {
        GitHubCheckResults::Connected(Vec::new())
    } else if let Some(connection) = connection {
        GitHubCheckResults::Connected(
            super::github_check_runs::latest_github_check_runs(
                conn,
                repo_id,
                connection.github_repository_id,
                &tested_oids,
            )
            .await?,
        )
    } else {
        GitHubCheckResults::Disconnected
    };
    let canonical_main_oid = if check_commit_request_ids.is_empty() {
        None
    } else {
        super::request_check_commits::canonical_main_oid(conn, repo_id).await?
    };
    Ok(RequestCheckResults {
        native_runs,
        github,
        withheld_from_github,
        canonical_main_oid,
    })
}

async fn requests_outside_anyone_view<C: ConnectionTrait>(
    conn: &C,
    request_ids: &[String],
    anyone: Option<&ViewId>,
) -> Result<Vec<String>, PostgresError> {
    let mut query = entities::request::Entity::find()
        .filter(entities::request::Column::Id.is_in(request_ids.iter().cloned()));
    if let Some(anyone) = anyone {
        query = query.filter(entities::request::Column::Audience.ne(anyone.as_str().to_string()));
    }
    Ok(query
        .all(conn)
        .await
        .map_err(PostgresError::internal)?
        .into_iter()
        .map(|row| row.id)
        .collect())
}

async fn start_runs(
    tx: &DatabaseTransaction,
    revisions: &[WorkflowRevision],
    runs: Vec<Run>,
) -> Result<Vec<Run>, PostgresError> {
    let mut created = Vec::new();
    for run in runs {
        let revision = revisions
            .iter()
            .find(|revision| revision.digest() == run.workflow_revision_digest)
            .cloned()
            .ok_or_else(|| {
                PostgresError::invalid_input("request check run has no workflow revision")
            })?;
        let stored = enqueue_run_in_transaction(tx, run, revision).await?;
        if stored.inserted {
            created.push(stored.run);
        }
    }
    Ok(created)
}

pub(super) async fn stop_auto_merge_for_evaluation(
    tx: &DatabaseTransaction,
    active_auto_merge: Option<super::request_auto_merge::StoredIntent>,
    request: &Request,
    evaluation: &RequestCheckEvaluation,
) -> Result<(), PostgresError> {
    if let Some(stored) = active_auto_merge
        && let Some(stopped) = stop_request_auto_merge_for_check_evaluation(
            request,
            &stored.intent,
            evaluation,
            super::request_auto_merge::automatic_event_id("stopped", &stored.intent.id),
        )?
    {
        super::request_auto_merge::persist_existing_auto_merge_mutation(tx, stored.model, &stopped)
            .await?;
    }
    Ok(())
}

pub(super) async fn queue_tested_commit_push(
    tx: &DatabaseTransaction,
    request: &Request,
    tested_oid: &str,
    now_unix: u64,
) -> Result<bool, PostgresError> {
    let Some(connection) = repository_github_connection(tx, &request.repo_id).await? else {
        return Ok(false);
    };
    queue_github_push(
        tx,
        &request.repo_id,
        &GitHubBranch::Request(request.id.clone()),
        Some(tested_oid),
        &GitHubPushDestination::of(&connection),
        now_unix,
    )
    .await?;
    Ok(true)
}

pub(super) async fn evaluation_for_head<C: ConnectionTrait>(
    conn: &C,
    request_id: &str,
    head_oid: &str,
) -> Result<Option<RequestCheckEvaluation>, PostgresError> {
    entities::request_check_evaluation::Entity::find_by_id((
        request_id.to_string(),
        head_oid.to_string(),
    ))
    .one(conn)
    .await
    .map_err(PostgresError::internal)?
    .map(entities::request_check_evaluation::Model::try_into_domain)
    .transpose()
}

pub(super) async fn save_evaluation(
    tx: &DatabaseTransaction,
    evaluation: &RequestCheckEvaluation,
) -> Result<(), PostgresError> {
    let model = entities::request_check_evaluation::Model::from_domain(evaluation)?;
    entities::request_check_evaluation::Entity::insert(model.into_active_model())
        .on_conflict(
            OnConflict::columns([
                entities::request_check_evaluation::Column::RequestId,
                entities::request_check_evaluation::Column::HeadOid,
            ])
            .update_columns([
                entities::request_check_evaluation::Column::TestedOid,
                entities::request_check_evaluation::Column::ChangesGithubWorkflows,
                entities::request_check_evaluation::Column::State,
                entities::request_check_evaluation::Column::Message,
                entities::request_check_evaluation::Column::Checks,
                entities::request_check_evaluation::Column::CreatedAtUnix,
                entities::request_check_evaluation::Column::UpdatedAtUnix,
                entities::request_check_evaluation::Column::CheckPrivateMainOid,
                entities::request_check_evaluation::Column::CheckPublicBaseOid,
            ])
            .to_owned(),
        )
        .exec(tx)
        .await
        .map_err(PostgresError::internal)?;
    Ok(())
}

#[cfg(test)]
mod tests;
