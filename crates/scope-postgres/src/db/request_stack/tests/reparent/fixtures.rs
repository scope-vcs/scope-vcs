use super::*;
use crate::db::RecordRequestChecksCommand;
use scope_domain::requests::{RequestCheckContext, RequestCheckEvaluation};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

pub(super) fn main_base() -> RequestBase {
    RequestBase::Main {
        review_base_oid: "base".into(),
    }
}

pub(super) async fn set_main(store: &MetadataStore, main: &str) {
    store.db.execute_raw(Statement::from_sql_and_values(DatabaseBackend::Postgres,
        "INSERT INTO scope_git_heads (repo_id, head_oid, push_sequence, change_version, frontier_digest)
         VALUES ('owner/repo', $1, 1, 1, 'sha256-main-frontier')
         ON CONFLICT (repo_id) DO UPDATE SET head_oid = excluded.head_oid",
        [main.into()])).await.unwrap();
}

pub(super) async fn selected_parent_facts(
    store: &MetadataStore,
    child: &Request,
    base: &RequestBase,
) -> Option<RequestBasePublicationFacts> {
    let mut selected = child.clone();
    selected.base = base.clone();
    parent_facts(store, &selected).await
}

pub(super) async fn context_for(
    store: &MetadataStore,
    child: &Request,
    base: RequestBase,
    facts: Option<&RequestBasePublicationFacts>,
    key: &str,
) -> RequestRevisionContextPublication {
    RequestRevisionContextPublication {
        operation_key: key.into(),
        expected_revision_id: store
            .requests()
            .latest_request_revision(&child.id)
            .await
            .unwrap()
            .unwrap()
            .id,
        expected_head_oid: child.head_oid.clone(),
        expected_base_version: child.base_version,
        base,
        expected_parent_base_version: match facts {
            Some(RequestBasePublicationFacts::Request(parent)) => Some(parent.base_version),
            _ => None,
        },
    }
}

pub(super) async fn publish(
    store: &MetadataStore,
    child: &Request,
    mut publication: RecordRequestRevisionInput,
    facts: Option<RequestBasePublicationFacts>,
    same_head: bool,
) -> Result<scope_domain::requests::RequestRevisionMutation, PostgresError> {
    if same_head {
        let context = context_for(
            store,
            child,
            publication.new_base.clone().unwrap(),
            facts.as_ref(),
            &publication.event_id,
        )
        .await;
        publication.git_snapshot = store
            .requests()
            .latest_request_revision(&child.id)
            .await
            .unwrap()
            .unwrap()
            .git_snapshot;
        store
            .requests()
            .record_request_revision_context(publication, context, facts, Vec::new())
            .await
    } else {
        store
            .requests()
            .record_request_revision_with_stack(publication, child.base_version, facts, Vec::new())
            .await
    }
}

pub(super) async fn seed_checks(
    store: &MetadataStore,
    child: &Request,
    now: u64,
) -> RequestCheckEvaluation {
    let revision = store
        .requests()
        .latest_request_revision(&child.id)
        .await
        .unwrap()
        .unwrap();
    let context = RequestCheckContext::for_revision(child, &revision).unwrap();
    let evaluation = RequestCheckEvaluation::no_checks(&context, now).unwrap();
    store
        .requests()
        .record_request_checks(RecordRequestChecksCommand {
            repository_incarnation: store
                .repositories()
                .repository_record(&child.repo_id)
                .await
                .unwrap()
                .unwrap()
                .incarnation(),
            expected_canonical_main_oid: store
                .requests()
                .request_check_base(&child.repo_id)
                .await
                .unwrap(),
            evaluation: evaluation.clone(),
            revisions: Vec::new(),
            runs: Vec::new(),
            push_to_github: false,
        })
        .await
        .unwrap();
    evaluation
}

pub(super) async fn reference_count(store: &MetadataStore, revision_id: &str) -> i64 {
    store
        .db
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT count(*) AS count FROM scope_object_references
         WHERE ref_kind = 'request_revision_snapshot' AND ref_id = $1",
            [revision_id.into()],
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get("", "count")
        .unwrap()
}
