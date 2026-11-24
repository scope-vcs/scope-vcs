use super::*;
use crate::{
    db::{
        AddRequestInviteeCommand, MetadataStore, OpenRequestFromAskCommand,
        repository_access::repository_access,
        requests::tests::{postgres_store, public_start_input, start_public_request},
        test_support::request_fixtures::closed,
    },
    error::PostgresErrorKind,
};
use scope_domain::{
    github::account::{GitHubAccount, GitHubAccountType},
    requests::{
        CloseRequestInput, OpenRequestFromAskInput, RequestAsk, RequestCloseAs, RequestLinks,
        StartRequestInput,
    },
    views::ViewId,
};

async fn open_ask(store: &MetadataStore, id: &str, github_issue_number: u64) {
    store
        .requests()
        .open_request_from_ask(OpenRequestFromAskCommand {
            input: OpenRequestFromAskInput {
                id: id.to_string(),
                repo_id: "owner/repo".to_string(),
                title: format!("Ask {github_issue_number}"),
                ask: RequestAsk {
                    github_account_id: 7,
                    github_issue_number,
                    source_url: format!(
                        "https://github.com/owner/repo/issues/{github_issue_number}"
                    ),
                    text_markdown: String::new(),
                    asked_at_unix: 1,
                },
                view: ViewId::public(),
                main_oid: "base".to_string(),
                event_id: format!("event_{id}"),
                now_unix: 2,
            },
            asker: GitHubAccount::new(7, "tomasz-k", GitHubAccountType::User, None).unwrap(),
        })
        .await
        .unwrap();
}

fn start(id: &str, name: &str, author_user_id: &str, now_unix: u64) -> StartRequestInput {
    StartRequestInput {
        id: id.to_string(),
        name: name.to_string(),
        author_user_id: author_user_id.to_string(),
        event_id: format!("event_started_{id}"),
        now_unix,
        ..public_start_input()
    }
}

async fn links_of_ask(store: &MetadataStore, viewer_user_id: Option<&str>) -> RequestLinks {
    let ask = store
        .requests()
        .request_by_id("req_ask")
        .await
        .unwrap()
        .unwrap();
    let repository = repository_access(store.db.as_ref(), "owner/repo", viewer_user_id)
        .await
        .unwrap()
        .unwrap();
    stored_links(store.db.as_ref(), &ask, viewer_user_id)
        .await
        .unwrap()
        .seen_by(&ask, &repository.access, viewer_user_id, &repository.views)
}

fn ids(linked: &[LinkedRequest]) -> Vec<&str> {
    linked
        .iter()
        .map(|linked| linked.request.id.as_str())
        .collect()
}

#[tokio::test]
async fn links_and_a_named_close_are_stored_for_the_viewers_who_may_see_them() {
    let store = postgres_store();
    let requests = store.requests();
    start_public_request(&store).await;
    open_ask(&store, "req_ask", 142).await;
    open_ask(&store, "req_hidden", 7).await;
    store
        .db
        .execute_unprepared(
            "UPDATE scope_requests SET audience = 'private' WHERE id = 'req_hidden'",
        )
        .await
        .unwrap();

    let error = requests
        .start_request_resolving(
            start("req_2", "probe-timeout", "user_public", 4),
            &["req_hidden".to_string()],
        )
        .await
        .unwrap_err();
    assert_eq!(error.kind, PostgresErrorKind::NotFound);
    assert!(requests.request_by_id("req_2").await.unwrap().is_none());

    requests
        .start_request_resolving(
            start("req_2", "probe-timeout", "user_public", 4),
            &["req_ask".to_string()],
        )
        .await
        .unwrap();
    requests
        .edit_request_links("req_1", "user_public", &["req_ask".to_string()], &[], 6)
        .await
        .unwrap();
    requests
        .add_request_invitee(AddRequestInviteeCommand {
            request_id: "req_1".to_string(),
            actor_user_id: "user_public".to_string(),
            target_handle: "guest".to_string(),
            now_unix: 6,
        })
        .await
        .unwrap();
    for (viewer_user_id, resolved_by) in [
        (None, vec![]),
        (Some("user_guest"), vec!["req_1"]),
        (Some("user_public"), vec!["req_2", "req_1"]),
    ] {
        let links = links_of_ask(&store, viewer_user_id).await;
        assert_eq!(ids(&links.resolved_by), resolved_by, "{viewer_user_id:?}");
    }

    requests
        .start_request(start("req_3", "owner-fix", "user_owner", 7))
        .await
        .unwrap();
    let tx = requests
        .begin_request("req_ask", "user_owner")
        .await
        .unwrap();
    let fixed_by = tx
        .named_requests(&["req_3".to_string()])
        .await
        .unwrap()
        .pop();
    drop(tx);
    closed(
        &requests,
        CloseRequestInput {
            request_id: "req_ask".to_string(),
            actor_user_id: "user_owner".to_string(),
            actor_is_maintainer: true,
            reason: Some(RequestCloseAs::Completed { fixed_by }),
            event_id: "event_closed".to_string(),
            now_unix: 8,
        },
    )
    .await;
    for (viewer_user_id, close_named) in [(None, None), (Some("user_owner"), Some("req_3"))] {
        let links = links_of_ask(&store, viewer_user_id).await;
        assert_eq!(
            links
                .close_named
                .as_ref()
                .map(|named| named.request.id.as_str()),
            close_named,
            "{viewer_user_id:?}"
        );
    }

    requests
        .edit_request_links("req_1", "user_public", &[], &["req_ask".to_string()], 9)
        .await
        .unwrap();
    for (request_id, actor_user_id) in [("req_2", "user_public"), ("req_3", "user_owner")] {
        closed(
            &requests,
            CloseRequestInput {
                request_id: request_id.to_string(),
                actor_user_id: actor_user_id.to_string(),
                actor_is_maintainer: false,
                reason: None,
                event_id: format!("event_deleted_{request_id}"),
                now_unix: 10,
            },
        )
        .await;
    }
    assert_eq!(
        links_of_ask(&store, Some("user_owner")).await,
        RequestLinks::default()
    );
    let ask = requests.request_by_id("req_ask").await.unwrap().unwrap();
    assert_eq!(ask.close_named_request_id(), None);
}
