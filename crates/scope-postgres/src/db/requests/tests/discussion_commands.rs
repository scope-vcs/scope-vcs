use super::*;

#[tokio::test]
async fn line_anchor_columns_are_all_set_or_all_null() {
    use sea_orm::ConnectionTrait;
    let store = postgres_store();
    start_public_request(&store).await;
    let revision = store
        .requests()
        .record_request_revision(RecordRequestRevisionInput {
            new_base: None,
            new_base_stack_depth: None,
            request_id: "req_1".into(),
            actor_user_id: "user_public".into(),
            expected_old_head_oid: Some("head".into()),
            new_head_oid: "head_lines".into(),
            git_facts: RequestRevisionGitFacts {
                contains_old_head: true,
                contained_main_oid: None,
                contained_main_descends_from_base: false,
            },
            event_id: "revision_lines".into(),
            body: None,
            now_unix: 3,
        })
        .await
        .unwrap()
        .revision;
    crate::db::test_support::request_fixtures::discussion_created(
        &store.requests(),
        scope_domain::requests::CreateRequestDiscussionInput {
            request_id: "req_1".into(),
            id: "discussion_lines".into(),
            actor_user_id: "user_public".into(),
            client_discussion_id: "client_lines".into(),
            body_markdown: "Line review".into(),
            anchor: None,
            now_unix: 4,
        },
    )
    .await;
    let set_lines = |start_line: &str, end_line: &str| {
        format!(
            "UPDATE scope_request_discussions SET revision_id = '{}', path = '/notes.txt', \
             line_side = 'New', start_line = {start_line}, end_line = {end_line} \
             WHERE id = 'discussion_lines'",
            revision.id
        )
    };

    for (start_line, end_line) in [("NULL", "2"), ("1", "NULL"), ("NULL", "NULL")] {
        let error = store
            .db
            .execute_unprepared(&set_lines(start_line, end_line))
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("scope_request_discussion_line_anchor"),
            "{start_line}-{end_line}: {error}"
        );
    }
    store
        .db
        .execute_unprepared(&set_lines("1", "2"))
        .await
        .unwrap();
}
