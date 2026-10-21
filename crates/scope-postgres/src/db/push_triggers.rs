use super::{
    GeneratedIdKind, RunStore, entities, generated_ids::generate_id,
    native_runs::lock_native_runs_availability, outbox::ClaimedOutboxJob,
    runs::enqueue_run_in_transaction,
};
use crate::error::PostgresError;
use scope_domain::{
    repository::git::GitHead,
    runs::{
        run::Run,
        source::{RunSource, RunTrigger},
        trigger::{
            PushTriggerCheck, PushTriggerEvaluation, PushTriggerEvaluationState, PushTriggerInput,
        },
        workflow::revision::WorkflowRevision,
    },
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseTransaction, EntityTrait,
    IntoActiveModel, QueryFilter, QueryOrder, QuerySelect, TransactionTrait,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};

pub(super) const JOB_KIND: &str = "push_main_trigger_evaluation";

#[derive(Deserialize)]
struct PushMainTriggerJobPayload {
    workflow_schema_version: u8,
    head: GitHead,
    input: PushTriggerInput,
}

pub async fn enqueue_push_main_trigger_evaluation<C>(
    conn: &C,
    repo_id: &str,
    head: &GitHead,
    input: &PushTriggerInput,
    now_unix: u64,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    if head.head_oid != input.head_oid {
        return Err(PostgresError::invalid_input(
            "push trigger input does not match the accepted Git head",
        ));
    }
    let evaluation = PushTriggerEvaluation::for_push(repo_id, head.change_version, input, now_unix)
        .map_err(PostgresError::from)?;
    entities::push_trigger_evaluation::Model::from_domain(&evaluation)?
        .into_active_model()
        .insert(conn)
        .await?;
    if evaluation.state != PushTriggerEvaluationState::Pending {
        return Ok(());
    }
    RunSource::accepted_git_head(repo_id, head.clone()).map_err(PostgresError::from)?;
    let job = entities::outbox_job::Model::push_main_trigger_evaluation(
        generate_id(GeneratedIdKind::OutboxJob)?,
        JOB_KIND,
        repo_id,
        head,
        input,
        now_unix,
    )?;
    entities::outbox_job::Entity::insert(job.into_active_model())
        .exec(conn)
        .await?;
    Ok(())
}

pub(super) async fn evaluate<C>(
    conn: &C,
    job: &ClaimedOutboxJob,
    now_unix: u64,
) -> Result<Vec<String>, PostgresError>
where
    C: ConnectionTrait + TransactionTrait<Transaction = DatabaseTransaction>,
{
    let payload = load_job_payload(conn, &job.id).await?;
    let change_version = u64::try_from(job.repo_version)
        .map_err(|_| PostgresError::internal_message("push trigger change version is negative"))?;
    let tx = conn.begin().await?;
    let Some(model) = entities::push_trigger_evaluation::Entity::find_by_id((
        job.repo_id.clone(),
        job.repo_version,
    ))
    .lock_exclusive()
    .one(&tx)
    .await?
    else {
        tx.commit().await?;
        return Ok(Vec::new());
    };
    let mut evaluation = model.try_into_domain()?;
    if evaluation.state != PushTriggerEvaluationState::Pending {
        tx.commit().await?;
        return Ok(Vec::new());
    }
    let native_runs = lock_native_runs_availability(&tx, &job.repo_id).await?;
    let revisions = if !native_runs.is_available() {
        Vec::new()
    } else if let Some(message) = &payload.input.configuration_error {
        evaluation
            .configuration_error(message.clone(), now_unix)
            .map_err(PostgresError::from)?;
        Vec::new()
    } else {
        let files = payload
            .input
            .workflows
            .iter()
            .map(|workflow| (workflow.path.as_str(), workflow.bytes.as_slice()));
        match scope_run_config::parse_workflow_set(&job.repo_id, files) {
            Ok(revisions) => revisions,
            Err(error) => {
                evaluation
                    .configuration_error(error.to_string(), now_unix)
                    .map_err(PostgresError::from)?;
                Vec::new()
            }
        }
    };
    let revisions = revisions
        .into_iter()
        .filter(|revision| revision.definition().triggers().push_main())
        .collect::<Vec<_>>();
    let mut checks = Vec::new();
    let mut created_run_ids = Vec::new();
    if evaluation.state == PushTriggerEvaluationState::Pending {
        for revision in revisions {
            let path = revision.workflow().path().as_str().to_string();
            let run = push_main_run(&job.repo_id, change_version, &payload, &revision, now_unix)?;
            let stored = enqueue_run_in_transaction(&tx, run, revision).await?;
            if stored.inserted {
                created_run_ids.push(stored.run.id.clone());
            }
            checks.push(PushTriggerCheck {
                workflow_path: path,
                workflow_name: stored.run.workflow.path().name().to_string(),
                run_id: stored.run.id,
            });
        }
        evaluation
            .succeed(checks, now_unix)
            .map_err(PostgresError::from)?;
    }
    save_evaluation(&tx, &evaluation).await?;
    tx.commit().await?;
    Ok(created_run_ids)
}

async fn load_job_payload<C>(
    conn: &C,
    job_id: &str,
) -> Result<PushMainTriggerJobPayload, PostgresError>
where
    C: ConnectionTrait,
{
    let payload = entities::outbox_job::Entity::find_by_id(job_id.to_string())
        .one(conn)
        .await?
        .ok_or_else(|| PostgresError::not_found("push trigger outbox job not found"))?;
    let payload: PushMainTriggerJobPayload =
        serde_json::from_value(payload.payload).map_err(PostgresError::internal)?;
    if payload.workflow_schema_version
        != entities::outbox_job::PUSH_MAIN_TRIGGER_WORKFLOW_SCHEMA_VERSION
    {
        return Err(PostgresError::internal_message(
            "unsupported push trigger workflow schema version",
        ));
    }
    Ok(payload)
}

pub(super) async fn mark_terminal_failure(
    tx: &DatabaseTransaction,
    job: &ClaimedOutboxJob,
    error: String,
    now_unix: u64,
) -> Result<(), PostgresError> {
    let Some(model) = entities::push_trigger_evaluation::Entity::find_by_id((
        job.repo_id.clone(),
        job.repo_version,
    ))
    .lock_exclusive()
    .one(tx)
    .await?
    else {
        return Ok(());
    };
    let mut evaluation = model.try_into_domain()?;
    if evaluation.state == PushTriggerEvaluationState::Pending {
        evaluation
            .fail(error, now_unix)
            .map_err(PostgresError::from)?;
        save_evaluation(tx, &evaluation).await?;
    }
    Ok(())
}

async fn save_evaluation(
    tx: &DatabaseTransaction,
    evaluation: &PushTriggerEvaluation,
) -> Result<(), PostgresError> {
    entities::push_trigger_evaluation::Entity::update(
        entities::push_trigger_evaluation::Model::from_domain(evaluation)?
            .into_active_model()
            .reset_all(),
    )
    .exec(tx)
    .await?;
    Ok(())
}

fn push_main_run(
    repo_id: &str,
    change_version: u64,
    payload: &PushMainTriggerJobPayload,
    revision: &WorkflowRevision,
    now_unix: u64,
) -> Result<Run, PostgresError> {
    let idempotency_key = format!(
        "push-main:{change_version}:{}:{}",
        payload.input.head_oid,
        revision.workflow().path().as_str()
    );
    let digest = Sha256::digest(format!("{repo_id}\0{idempotency_key}").as_bytes());
    let source = RunSource::accepted_git_head(repo_id, payload.head.clone())?;
    Ok(Run::new(
        format!("run_push_{}", hex::encode(digest)),
        idempotency_key,
        revision.workflow().clone(),
        revision.digest(),
        RunTrigger::PushMain,
        None,
        source,
        now_unix,
    )?)
}

impl RunStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "push_trigger_evaluation"))]
    pub async fn push_trigger_evaluation(
        &self,
        repository_id: &str,
        head_oid: &str,
    ) -> Result<Option<PushTriggerEvaluation>, PostgresError> {
        entities::push_trigger_evaluation::Entity::find()
            .filter(entities::push_trigger_evaluation::Column::RepoId.eq(repository_id.to_string()))
            .filter(entities::push_trigger_evaluation::Column::HeadOid.eq(head_oid.to_string()))
            .order_by_desc(entities::push_trigger_evaluation::Column::ChangeVersion)
            .one(self.db.as_ref())
            .await?
            .map(entities::push_trigger_evaluation::Model::try_into_domain)
            .transpose()
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "runs_by_ids"))]
    pub async fn runs_by_ids(&self, run_ids: &[String]) -> Result<Vec<Run>, PostgresError> {
        if run_ids.is_empty() {
            return Ok(Vec::new());
        }
        entities::run::Entity::find()
            .filter(entities::run::Column::Id.is_in(run_ids.to_vec()))
            .all(self.db.as_ref())
            .await?
            .into_iter()
            .map(entities::run::Model::try_into_domain)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::MetadataStore;
    use scope_domain::{account::UserAccount, runs::trigger::PushWorkflowFile, views::ViewId};

    #[tokio::test]
    async fn evaluation_uses_each_pinned_head_and_enqueues_once() {
        let target = crate::db::TestDatabaseTarget::required().unwrap();
        let store = MetadataStore::connect_fresh_for_tests(&target).unwrap();
        let repo_id = seed_repo(&store).await;
        let head_oid = "1111111111111111111111111111111111111111";
        let jobs = "  backend:\n    steps: [{ name: Backend, run: \"true\" }]\n  web:\n    needs: [backend]\n    steps: [{ name: Web, run: \"true\" }]\n";
        let workflows = vec![workflow_file("test", jobs)];
        enqueue(&store, &repo_id, head_oid, 1, workflows).await;
        let later_head_oid = "3333333333333333333333333333333333333333";
        enqueue(&store, &repo_id, later_head_oid, 3, Vec::new()).await;

        let summary = store
            .jobs()
            .run_ready_outbox_jobs("push-worker", 10, &|| Ok(now()))
            .await
            .unwrap();
        assert_eq!(summary.failed, 0);
        let evaluation = store
            .runs()
            .push_trigger_evaluation(&repo_id, head_oid)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(evaluation.state, PushTriggerEvaluationState::Succeeded);
        assert_eq!(evaluation.checks.len(), 1);
        assert_eq!(summary.created_runs.len(), 1);
        assert_eq!(summary.created_runs[0].repo_id, repo_id);
        assert_eq!(summary.created_runs[0].run_id, evaluation.checks[0].run_id);
        let run = store
            .runs()
            .run(&evaluation.checks[0].run_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(run.trigger, RunTrigger::PushMain);
        assert_eq!(run.source.git_oid(), head_oid);
        let jobs = entities::run_job::Entity::find()
            .filter(entities::run_job::Column::RunId.eq(run.id.clone()))
            .all(store.db.as_ref())
            .await
            .unwrap();
        assert_eq!(jobs.len(), 2);
        let later = store
            .runs()
            .push_trigger_evaluation(&repo_id, later_head_oid)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(later.state, PushTriggerEvaluationState::Succeeded);
        assert!(later.checks.is_empty());

        let replay = store
            .jobs()
            .run_ready_outbox_jobs("push-worker", 10, &|| Ok(now()))
            .await
            .unwrap();
        assert!(replay.created_runs.is_empty());
        assert_eq!(store.runs().runs_by_ids(&[run.id]).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn idempotent_trigger_enqueue_emits_no_created_run_ids() {
        let target = crate::db::TestDatabaseTarget::required().unwrap();
        let store = MetadataStore::connect_fresh_for_tests(&target).unwrap();
        let repo_id = seed_repo(&store).await;
        let head_oid = "7777777777777777777777777777777777777777";
        enqueue(
            &store,
            &repo_id,
            head_oid,
            7,
            vec![workflow_file("idempotent", CHECKS)],
        )
        .await;
        let job = entities::outbox_job::Entity::find()
            .filter(entities::outbox_job::Column::RepoId.eq(repo_id.clone()))
            .filter(entities::outbox_job::Column::Kind.eq(JOB_KIND))
            .one(store.db.as_ref())
            .await
            .unwrap()
            .unwrap();
        let claimed = ClaimedOutboxJob {
            id: job.id,
            kind: job.kind,
            repo_id: job.repo_id,
            repo_version: job.repo_version,
            attempts: job.attempts,
            queued_trace: Default::default(),
        };
        let payload = load_job_payload(store.db.as_ref(), &claimed.id)
            .await
            .unwrap();
        let mut revisions = scope_run_config::parse_workflow_set(
            &repo_id,
            payload
                .input
                .workflows
                .iter()
                .map(|workflow| (workflow.path.as_str(), workflow.bytes.as_slice())),
        )
        .unwrap();
        let revision = revisions.remove(0);
        let run = push_main_run(&repo_id, 7, &payload, &revision, now()).unwrap();
        store.runs().enqueue_run(run, revision).await.unwrap();

        let replayed_enqueue = evaluate(store.db.as_ref(), &claimed, now()).await.unwrap();
        let replayed_evaluation = evaluate(store.db.as_ref(), &claimed, now()).await.unwrap();

        assert!(replayed_enqueue.is_empty());
        assert!(replayed_evaluation.is_empty());
    }

    #[tokio::test]
    async fn invalid_workflow_is_a_configuration_error() {
        let target = crate::db::TestDatabaseTarget::required().unwrap();
        let store = MetadataStore::connect_fresh_for_tests(&target).unwrap();
        let repo_id = seed_repo(&store).await;
        let head_oid = "2222222222222222222222222222222222222222";
        let broken = PushWorkflowFile::new(
            "/.scope/runs/broken.yml",
            b"name: broken\nunknown: true\n".to_vec(),
        )
        .unwrap();
        enqueue(&store, &repo_id, head_oid, 2, vec![broken]).await;
        store
            .jobs()
            .run_ready_outbox_jobs("push-worker", 10, &|| Ok(now()))
            .await
            .unwrap();

        let evaluation = store
            .runs()
            .push_trigger_evaluation(&repo_id, head_oid)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            evaluation.state,
            PushTriggerEvaluationState::ConfigurationError
        );
        assert!(
            evaluation
                .message
                .unwrap()
                .contains("workflow YAML is invalid")
        );
        assert!(evaluation.checks.is_empty());
        let protected = entities::source_blob_cleanup_job::Entity::find()
            .all(store.db.as_ref())
            .await
            .unwrap();
        assert!(protected.is_empty());
        let cleanup = store
            .cleanup()
            .source_blob_cleanup_batch(
                now() + crate::db::cleanup_queue::queue::SOURCE_BLOB_DELETE_GRACE_SECONDS + 1,
            )
            .await
            .unwrap();
        assert!(cleanup.pending.is_empty());
    }

    #[tokio::test]
    async fn malformed_payload_terminal_failure_stops_retries() {
        let target = crate::db::TestDatabaseTarget::required().unwrap();
        let store = MetadataStore::connect_fresh_for_tests(&target).unwrap();
        let repo_id = seed_repo(&store).await;
        let head_oid = "5555555555555555555555555555555555555555";
        let workflows = vec![workflow_file("malformed", CHECKS)];
        enqueue(&store, &repo_id, head_oid, 5, workflows).await;
        store
            .db
            .execute_raw(sea_orm::Statement::from_sql_and_values(
                sea_orm::DatabaseBackend::Postgres,
                "UPDATE scope_outbox_jobs SET payload = $2, attempts = 11 WHERE repo_id = $1",
                vec![
                    repo_id.clone().into(),
                    serde_json::json!({"workflow_schema_version": 5}).into(),
                ],
            ))
            .await
            .unwrap();
        let summary = store
            .jobs()
            .run_ready_outbox_jobs("worker", 10, &|| Ok(now()))
            .await
            .unwrap();
        assert_eq!(summary.failed, 1);
        assert_eq!(
            store
                .jobs()
                .outbox_job_counts_for_tests()
                .await
                .unwrap()
                .failed,
            1
        );
        let retry = store
            .jobs()
            .run_ready_outbox_jobs("worker", 10, &|| Ok(now() + 600))
            .await
            .unwrap();
        assert_eq!(retry.claimed, 0);

        let evaluation = store
            .runs()
            .push_trigger_evaluation(&repo_id, head_oid)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(evaluation.state, PushTriggerEvaluationState::Failed);
    }

    #[tokio::test]
    async fn returning_to_an_earlier_head_creates_a_new_evaluation_and_run() {
        let target = crate::db::TestDatabaseTarget::required().unwrap();
        let store = MetadataStore::connect_fresh_for_tests(&target).unwrap();
        let repo_id = seed_repo(&store).await;
        let head_oid = "4444444444444444444444444444444444444444";
        for change_version in [2, 4] {
            let workflows = vec![workflow_file("repeat", CHECKS)];
            enqueue(&store, &repo_id, head_oid, change_version, workflows).await;
        }

        let summary = store
            .jobs()
            .run_ready_outbox_jobs("push-worker", 10, &|| Ok(now()))
            .await
            .unwrap();
        assert_eq!(summary.failed, 0);
        let evaluation = store
            .runs()
            .push_trigger_evaluation(&repo_id, head_oid)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(evaluation.change_version, 4);
        assert_eq!(evaluation.checks.len(), 1);
        let runs = entities::run::Entity::find()
            .filter(entities::run::Column::RepoId.eq(repo_id))
            .all(store.db.as_ref())
            .await
            .unwrap();
        assert_eq!(runs.len(), 2);
        assert_ne!(runs[0].id, runs[1].id);
    }

    const CHECKS: &str = "  checks:\n    steps: [{ name: Test, run: \"true\" }]\n";

    fn workflow_file(name: &str, jobs: &str) -> PushWorkflowFile {
        let image = format!("alpine@sha256:{}", "a".repeat(64));
        let yaml = format!(
            "name: {name}\non: {{ push: true }}\ncontainer: {{ image: {image} }}\ntimeout: 1m\njobs:\n{jobs}"
        );
        PushWorkflowFile::new(format!("/.scope/runs/{name}.yml"), yaml.into_bytes()).unwrap()
    }

    async fn enqueue(
        store: &MetadataStore,
        repo_id: &str,
        head_oid: &str,
        change_version: u64,
        workflows: Vec<PushWorkflowFile>,
    ) {
        enqueue_push_main_trigger_evaluation(
            store.db.as_ref(),
            repo_id,
            &trigger_head(head_oid, change_version),
            &PushTriggerInput::new(head_oid, workflows, None).unwrap(),
            now(),
        )
        .await
        .unwrap();
    }

    async fn seed_repo(store: &MetadataStore) -> String {
        let owner = UserAccount {
            id: "user_owner".to_string(),
            handle: "owner".to_string(),
            email: "owner@example.com".to_string(),
            email_verified: true,
        };
        let mut catalog = crate::db::CatalogFixture::default();
        let repo = catalog
            .create_repository(&owner, "repo", ViewId::private())
            .unwrap()
            .clone();
        catalog.native_runs_accounts.insert(owner.id.clone());
        catalog.users.insert(owner.id.clone(), owner);
        store.admin().seed_catalog_for_tests(catalog).unwrap();
        repo.record.id
    }

    fn trigger_head(head_oid: &str, change_version: u64) -> GitHead {
        GitHead::new(head_oid.to_string(), change_version, change_version)
    }

    fn now() -> u64 {
        1_700_000_000
    }
}
