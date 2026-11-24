use super::*;
use crate::db::{
    MetadataStore, RequestListPageQuery, RequestQueuePageQuery,
    request_access::repo_by_id,
    requests::tests::{postgres_store, start_public_request},
};
use scope_domain::{
    github::account::GitHubAccountType,
    repository::access::RepositoryAccess,
    requests::{RequestAsk, RequestQueueSection, RequestWorkInput, start_request_work},
    views::{ViewId, Views},
};

fn command() -> OpenRequestFromAskCommand {
    OpenRequestFromAskCommand {
        input: OpenRequestFromAskInput {
            id: "req_ask".to_string(),
            repo_id: "owner/repo".to_string(),
            title: "Login times out after 30 seconds".to_string(),
            ask: RequestAsk {
                github_account_id: 7,
                github_issue_number: 142,
                source_url: "https://github.com/owner/repo/issues/142".to_string(),
                text_markdown: "Sign-in gives up before the server answers.".to_string(),
                asked_at_unix: 1,
            },
            view: ViewId::public(),
            main_oid: "base".to_string(),
            event_id: "event_asked".to_string(),
            now_unix: 5,
        },
        asker: GitHubAccount::new(7, "tomasz-k", GitHubAccountType::User, None).unwrap(),
    }
}

async fn queued_ids(store: &MetadataStore, section: RequestQueueSection) -> Vec<String> {
    let repo = repo_by_id(store.db.as_ref(), "owner/repo", "user_owner")
        .await
        .unwrap();
    store
        .requests()
        .request_queue_page(RequestQueuePageQuery {
            repo_id: "owner/repo",
            section,
            viewer_user_id: Some("user_owner"),
            access: repo.access,
            views: &repo.views,
            request_views: None,
            search: None,
            after: None,
            limit: 10,
            now_unix: 9,
        })
        .await
        .unwrap()
        .rows
        .into_iter()
        .map(|row| row.request.id)
        .collect()
}

#[tokio::test]
async fn an_asked_request_is_listed_for_every_reader_and_queued_only_for_its_worker() {
    let store = postgres_store();
    let requests = store.requests();
    start_public_request(&store).await;
    let opened = requests.open_request_from_ask(command()).await.unwrap();

    let listed = requests
        .request_list_page(RequestListPageQuery {
            repo_id: "owner/repo",
            viewer_user_id: None,
            access: RepositoryAccess::public(),
            views: &Views::builtin(),
            after_id: None,
            name: None,
            limit: 10,
        })
        .await
        .unwrap();
    assert_eq!(
        listed
            .iter()
            .map(|row| (row.id.as_str(), row.awaits_change))
            .collect::<Vec<_>>(),
        [("req_ask", true)]
    );
    for section in [RequestQueueSection::Active, RequestQueueSection::Unclaimed] {
        assert!(queued_ids(&store, section).await.is_empty(), "{section:?}");
    }

    let tx = requests
        .begin_request("req_ask", "user_owner")
        .await
        .unwrap();
    let taken = start_request_work(
        tx.request().clone(),
        tx.actor(),
        false,
        RequestWorkInput {
            request_id: "req_ask".to_string(),
            actor_user_id: "user_owner".to_string(),
            event_id: "event_work".to_string(),
            now_unix: 6,
        },
    )
    .unwrap();
    tx.save_work_change(&taken).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        queued_ids(&store, RequestQueueSection::Active).await,
        ["req_ask"]
    );

    let stored = requests
        .request_by_reference("owner/repo", &RequestReference::GitHubIssue(142))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.ask, opened.request.ask);
    assert_eq!(stored.author_user_id.as_deref(), Some("user_owner"));
    assert_eq!(
        requests
            .request_asker(&stored)
            .await
            .unwrap()
            .unwrap()
            .login,
        "tomasz-k"
    );

    let mut again = command();
    again.input.id = "req_ask_again".to_string();
    again.input.event_id = "event_asked_again".to_string();
    let error = requests.open_request_from_ask(again).await.unwrap_err();
    assert_eq!(error.message, "this GitHub issue already has a request");
}
