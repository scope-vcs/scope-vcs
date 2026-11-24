use super::*;
use crate::db::{
    MetadataStore, OpenRequestFromAskCommand,
    request_access::repo_by_id,
    requests::tests::{postgres_store, start_public_request},
    test_support::request_fixtures::{discussion_created, replied},
};
use scope_domain::{
    github::account::{GitHubAccount, GitHubAccountType},
    repository::access::RepositoryAccess,
    requests::{
        CreateRequestDiscussionInput, CreateRequestDiscussionReplyInput, OpenRequestFromAskInput,
        RequestAsk,
    },
};
use sea_orm::ConnectionTrait as _;

const ASKS: [(&str, u64, &str, &str, u64); 6] = [
    ("req_limits", 61, "Document the API rate limits", "", 10),
    (
        "req_export",
        151,
        "Export drops the header row",
        "The log shows error 142.",
        20,
    ),
    (
        "req_login",
        142,
        "Login times out",
        "Sign-in gives up early.",
        30,
    ),
    ("req_private", 7, "Rotate the signing key", "", 5),
    ("req_closed", 8, "Billing ignores the theme", "", 6),
    ("req_submitted", 9, "Search flickers", "", 7),
];

async fn seeded() -> MetadataStore {
    let store = postgres_store();
    let requests = store.requests();
    start_public_request(&store).await;
    for (id, github_issue_number, title, text_markdown, asked_at_unix) in ASKS {
        requests
            .open_request_from_ask(OpenRequestFromAskCommand {
                input: OpenRequestFromAskInput {
                    id: id.to_string(),
                    repo_id: "owner/repo".to_string(),
                    title: title.to_string(),
                    ask: RequestAsk {
                        github_account_id: 7,
                        github_issue_number,
                        source_url: format!("https://github.com/owner/repo/issues/{id}"),
                        text_markdown: text_markdown.to_string(),
                        asked_at_unix,
                    },
                    view: ViewId::public(),
                    main_oid: "base".to_string(),
                    event_id: format!("event_{id}"),
                    now_unix: 40,
                },
                asker: GitHubAccount::new(7, "tomasz-k", GitHubAccountType::User, None).unwrap(),
            })
            .await
            .unwrap();
    }
    store
        .db
        .execute_unprepared(
            "UPDATE scope_requests SET author_user_id = 'user_owner', author_role = 'Owner'
                 WHERE id = 'req_login';
             UPDATE scope_requests SET audience = 'private' WHERE id = 'req_private';
             UPDATE scope_requests SET closed_at_unix = 40, close_reason = 'duplicate'
                 WHERE id = 'req_closed';
             UPDATE scope_requests SET pushed_at_unix = 40, submitted_at_unix = 40
                 WHERE id = 'req_submitted';",
        )
        .await
        .unwrap();
    discussion_created(
        &requests,
        CreateRequestDiscussionInput {
            request_id: "req_export".to_string(),
            id: "discussion_export".to_string(),
            actor_user_id: "user_public".to_string(),
            client_discussion_id: "client_export".to_string(),
            body_markdown: "Same here.".to_string(),
            anchor: None,
            now_unix: 42,
        },
    )
    .await;
    replied(
        &requests,
        CreateRequestDiscussionReplyInput {
            request_id: "req_export".to_string(),
            discussion_id: "discussion_export".to_string(),
            id: "reply_export".to_string(),
            actor_user_id: "user_owner".to_string(),
            client_reply_id: "client_reply_export".to_string(),
            body_markdown: "Which version?".to_string(),
            reply_to_reply_id: None,
            now_unix: 43,
        },
        false,
    )
    .await;
    store
}

#[derive(Default)]
struct Ask<'a> {
    signed_out: bool,
    search: &'a str,
    working: RequestBacklogWorking,
    sort: RequestBacklogSort,
    after: Option<RequestBacklogCursor>,
    limit: Option<u64>,
}

async fn page(store: &MetadataStore, ask: Ask<'_>) -> RequestBacklogPage {
    let repo = repo_by_id(store.db.as_ref(), "owner/repo", "user_owner")
        .await
        .unwrap();
    let viewer = if ask.signed_out {
        RequestViewer::new(RepositoryAccess::public(), None, false)
    } else {
        RequestViewer::new(repo.access, Some("user_owner"), false)
    };
    store
        .requests()
        .request_backlog_page(RequestBacklogPageQuery {
            repo_id: "owner/repo",
            viewer: &viewer,
            views: &repo.views,
            request_views: None,
            search: RequestBacklogSearch::parse(ask.search).as_ref(),
            working: ask.working,
            sort: ask.sort,
            after: ask.after.as_ref(),
            limit: ask.limit.unwrap_or(10),
        })
        .await
        .unwrap()
}

fn ids(page: &RequestBacklogPage) -> Vec<&str> {
    page.rows
        .iter()
        .map(|row| row.request_id.as_str())
        .collect()
}

#[tokio::test]
async fn the_backlog_query_pages_searches_filters_and_sorts_what_the_viewer_reads() {
    let store = seeded().await;

    let all = page(&store, Ask::default()).await;
    assert_eq!(
        ids(&all),
        ["req_private", "req_limits", "req_export", "req_login"]
    );
    assert_eq!(all.total, 4);
    assert_eq!(
        all.rows[2],
        RequestBacklogRow {
            request_id: "req_export".to_string(),
            title: "Export drops the header row".to_string(),
            github_issue_number: 151,
            asker_login: "tomasz-k".to_string(),
            asked_at_unix: 20,
            worker_user_id: None,
            message_count: 2,
        }
    );
    assert_eq!(all.rows[3].worker_user_id.as_deref(), Some("user_owner"));
    assert_eq!(all.users["user_owner"].handle, "owner");

    let signed_out = page(
        &store,
        Ask {
            signed_out: true,
            ..Ask::default()
        },
    )
    .await;
    assert_eq!(ids(&signed_out), ["req_limits", "req_export", "req_login"]);
    assert_eq!(signed_out.total, 3);

    let newest = page(
        &store,
        Ask {
            sort: RequestBacklogSort::Newest,
            limit: Some(3),
            ..Ask::default()
        },
    )
    .await;
    assert_eq!(ids(&newest), ["req_login", "req_export", "req_limits"]);
    assert_eq!(newest.total, 4);
    let rest = page(
        &store,
        Ask {
            sort: RequestBacklogSort::Newest,
            after: Some(newest.rows[2].cursor()),
            ..Ask::default()
        },
    )
    .await;
    assert_eq!(ids(&rest), ["req_private"]);
    let after_first = page(
        &store,
        Ask {
            after: Some(all.rows[0].cursor()),
            limit: Some(2),
            ..Ask::default()
        },
    )
    .await;
    assert_eq!(ids(&after_first), ["req_limits", "req_export"]);

    for (search, expected) in [
        ("142", vec!["req_export", "req_login"]),
        ("#142", vec!["req_login"]),
        ("14", vec!["req_export"]),
        ("SIGN-IN", vec!["req_login"]),
        ("header", vec!["req_export"]),
        ("#999", vec![]),
        ("%", vec![]),
    ] {
        let found = page(
            &store,
            Ask {
                search,
                ..Ask::default()
            },
        )
        .await;
        assert_eq!(ids(&found), expected, "{search:?}");
        assert_eq!(found.total, expected.len() as u64, "{search:?}");
    }

    for (working, signed_out, expected) in [
        (
            RequestBacklogWorking::Nobody,
            false,
            vec!["req_private", "req_limits", "req_export"],
        ),
        (RequestBacklogWorking::Me, false, vec!["req_login"]),
        (RequestBacklogWorking::Me, true, vec![]),
    ] {
        let filtered = page(
            &store,
            Ask {
                working,
                signed_out,
                ..Ask::default()
            },
        )
        .await;
        assert_eq!(ids(&filtered), expected, "{working:?} {signed_out}");
    }
}
