use super::*;
use crate::db::github_pull_requests::{github_pull_request, queue_carrier_for_pushed_head};
use scope_domain::{github_connection::ConnectGitHubRepository, requests::GitHubPullRequestIntent};

async fn opt_in(store: &MetadataStore) {
    let repositories = store.repositories();
    repositories
        .connect_github_repository(
            ConnectGitHubRepository {
                repository_id: "owner/repo".into(),
                installation_id: 7,
                github_repository_id: 42,
                github_full_name: "octo/checks".into(),
                github_private: true,
                acknowledge_public: false,
                run_import_count: 0,
                user_id: "user_owner".into(),
                now_unix: 1,
            },
            async || Ok::<_, crate::error::PostgresError>(true),
        )
        .await
        .unwrap();
    repositories
        .set_github_review_bot_pull_requests("owner/repo", "user_owner", true, 1)
        .await
        .unwrap();
}

async fn intent(store: &MetadataStore, id: &str) -> GitHubPullRequestIntent {
    github_pull_request(store.db.as_ref(), id)
        .await
        .unwrap()
        .unwrap()
        .intent
}

#[tokio::test]
async fn closing_a_parent_moves_its_open_childrens_carriers_onto_main_in_the_same_transaction() {
    let store = crate::db::requests::tests::postgres_store();
    opt_in(&store).await;
    let parent = start(
        &store,
        "parent",
        RequestBase::Main {
            review_base_oid: "base".into(),
        },
        2,
    )
    .await;
    let child = start(&store, "child", pin(&store, &parent).await, 8).await;
    let db = store.db.as_ref();
    queue_carrier_for_pushed_head(db, &parent.id, &parent.head_oid, 12)
        .await
        .unwrap();
    queue_carrier_for_pushed_head(db, &child.id, &child.head_oid, 12)
        .await
        .unwrap();
    assert_eq!(
        intent(&store, &child.id).await,
        GitHubPullRequestIntent::Open {
            base: "scope/requests/parent".into(),
            head_oid: child.head_oid.clone(),
        }
    );
    let before = github_pull_request(db, &child.id).await.unwrap().unwrap();

    store
        .requests()
        .close_request(CloseRequestCommand {
            request_id: parent.id.clone(),
            actor_user_id: "user_owner".into(),
            event_id: "close".into(),
            now_unix: 15,
        })
        .await
        .unwrap();

    let moved = github_pull_request(db, &child.id).await.unwrap().unwrap();
    assert_eq!(
        moved.intent,
        GitHubPullRequestIntent::Open {
            base: "main".into(),
            head_oid: child.head_oid.clone(),
        }
    );
    assert!(moved.intent_sequence > before.intent_sequence);
    assert_eq!(
        intent(&store, &parent.id).await,
        GitHubPullRequestIntent::Close
    );
}
