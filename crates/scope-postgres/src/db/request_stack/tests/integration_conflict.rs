use super::*;
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

#[tokio::test]
async fn integration_conflict_marker_fences_main_and_request_then_propagates_and_clears_on_repair()
{
    let store = super::super::super::requests::tests::postgres_store();
    let parent = start(
        &store,
        "parent",
        RequestBase::Main {
            review_base_oid: "base".into(),
        },
        2,
    )
    .await;
    let child = start(&store, "child", pin(&store, &parent).await, 6).await;
    let main = "d".repeat(40);
    store.db.execute_raw(Statement::from_sql_and_values(DatabaseBackend::Postgres,
        "INSERT INTO scope_git_heads (repo_id, head_oid, push_sequence, change_version, frontier_digest) VALUES ($1, $2, 1, 1, $3)",
        [parent.repo_id.clone().into(), main.clone().into(), "sha256-main-frontier".into()])).await.unwrap();
    for (head, version, expected_main) in [
        ("stale_head", parent.base_version, main.as_str()),
        (
            parent.head_oid.as_str(),
            parent.base_version + 1,
            main.as_str(),
        ),
        (parent.head_oid.as_str(), parent.base_version, "stale_main"),
    ] {
        let error = store
            .requests()
            .mark_request_integration_conflict(
                &parent.id,
                "user_owner",
                head,
                version,
                expected_main,
                10,
            )
            .await
            .unwrap_err();
        assert_eq!(error.kind, crate::error::PostgresErrorKind::Conflict);
        assert_eq!(
            load(&store, &parent.id).await.stack_status,
            RequestStackStatus::Main
        );
    }
    let blocked = store
        .requests()
        .mark_request_integration_conflict(
            &parent.id,
            "user_owner",
            &parent.head_oid,
            parent.base_version,
            &main,
            11,
        )
        .await
        .unwrap();
    assert_eq!(
        blocked.stack_status,
        RequestStackStatus::IntegrationConflict
    );
    assert_eq!(blocked.base, parent.base);
    assert_eq!(blocked.head_oid, parent.head_oid);
    assert_eq!(
        load(&store, &child.id).await.stack_status,
        RequestStackStatus::AncestorBlocked
    );
    let unchanged = store
        .requests()
        .record_request_revision_with_stack(
            input(&blocked, &blocked.head_oid, "unchanged", 12),
            blocked.base_version,
            None,
            vec![fact(&blocked.head_oid, &child, true)],
        )
        .await
        .unwrap()
        .request;
    assert_eq!(
        unchanged.stack_status,
        RequestStackStatus::IntegrationConflict
    );
    let publication = input(&unchanged, "repaired_parent", "repair", 13);
    let child_facts = vec![fact(&publication.new_head_oid, &child, true)];
    let repaired = store
        .requests()
        .record_request_revision_with_stack(publication, unchanged.base_version, None, child_facts)
        .await
        .unwrap()
        .request;
    assert_eq!(repaired.stack_status, RequestStackStatus::Main);
    assert_eq!(
        load(&store, &child.id).await.stack_status,
        RequestStackStatus::ParentExtended
    );
}
