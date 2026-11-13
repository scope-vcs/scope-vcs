use super::*;

pub(super) async fn request_check_context(store: &crate::db::MetadataStore) -> RequestCheckContext {
    let requests = store.requests();
    let request = requests.request_for_tests("req_1").await.unwrap().unwrap();
    let revision = requests
        .request_revision_window("req_1", None, 1)
        .await
        .unwrap()
        .revisions
        .into_iter()
        .next()
        .unwrap();
    RequestCheckContext::for_revision(&request, &revision).unwrap()
}

pub(super) async fn open_request_with_revision(store: &crate::db::MetadataStore) -> String {
    start_public_request(store).await;
    store
        .requests()
        .submit_request(SubmitRequestCommand {
            request_id: "req_1".into(),
            actor_user_id: "user_public".into(),
            event_id: "submitted".into(),
            now_unix: 4,
        })
        .await
        .unwrap();
    store
        .requests()
        .record_request_revision(RecordRequestRevisionInput {
            new_base: None,
            new_base_stack_depth: None,
            request_id: "req_1".into(),
            actor_user_id: "user_public".into(),
            actor_can_edit: true,
            expected_old_head_oid: Some("head".into()),
            new_head_oid: "a".repeat(40),
            git_facts: RequestRevisionGitFacts {
                contains_old_head: true,
                contained_main_oid: Some("f".repeat(40)),
                contained_main_descends_from_base: true,
            },
            event_id: "revision_current".into(),
            body: None,
            now_unix: 5,
        })
        .await
        .unwrap()
        .revision
        .id
}

pub(super) fn authorize(
    intent_id: &str,
    event_id: &str,
    revision_id: &str,
    now_unix: u64,
) -> AuthorizeRequestAutoMergeCommand {
    AuthorizeRequestAutoMergeCommand {
        request_id: "req_1".into(),
        actor_user_id: "user_owner".into(),
        expected_revision_id: revision_id.into(),
        expected_head_oid: "a".repeat(40),
        intent_id: intent_id.into(),
        event_id: event_id.into(),
        now_unix,
    }
}

pub(super) async fn record_configuration_error(store: &crate::db::MetadataStore) {
    store
        .requests()
        .record_request_checks(RecordRequestChecksCommand {
            expected_canonical_main_oid: store
                .requests()
                .request_check_base("owner/repo")
                .await
                .unwrap(),
            repository_incarnation: store
                .repositories()
                .repository_record("owner/repo")
                .await
                .unwrap()
                .unwrap()
                .incarnation(),
            evaluation: RequestCheckEvaluation::configuration_error(
                &request_check_context(store).await,
                "invalid request workflow",
                6,
            )
            .unwrap(),
            push_to_github: false,
        })
        .await
        .unwrap();
}
