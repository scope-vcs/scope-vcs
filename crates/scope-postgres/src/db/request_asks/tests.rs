use super::*;
use crate::db::{
    MetadataStore, RequestListPageQuery, RequestQueuePageQuery,
    request_access::repo_by_id,
    request_navigation::navigation,
    requests::tests::{postgres_store, start_public_request},
};
use scope_domain::{
    github::account::GitHubAccountType,
    repository::access::RepositoryAccess,
    requests::{
        RecordRequestRevisionInput, RequestAsk, RequestQueueSection, RequestRevisionGitFacts,
        RequestWorkInput, start_request_work,
    },
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
    queued_ids_for(store, section, "user_owner").await
}

async fn queued_ids_for(
    store: &MetadataStore,
    section: RequestQueueSection,
    viewer_user_id: &str,
) -> Vec<String> {
    let repo = repo_by_id(store.db.as_ref(), "owner/repo", viewer_user_id)
        .await
        .unwrap();
    store
        .requests()
        .request_queue_page(RequestQueuePageQuery {
            repo_id: "owner/repo",
            section,
            viewer_user_id: Some(viewer_user_id),
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
        .request_by_github_issue_number("owner/repo", 142)
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

    let parent_head = "a".repeat(40);
    let pin = requests
        .record_request_revision(RecordRequestRevisionInput {
            new_base: None,
            new_base_stack_depth: None,
            request_id: "req_1".to_string(),
            actor_user_id: "user_public".to_string(),
            expected_old_head_oid: Some("head".to_string()),
            new_head_oid: parent_head.clone(),
            git_facts: RequestRevisionGitFacts {
                contains_old_head: true,
                contained_main_oid: None,
                contained_main_descends_from_base: false,
            },
            event_id: "event_parent_revision".to_string(),
            body: None,
            now_unix: 4,
        })
        .await
        .unwrap()
        .revision
        .id;
    store
        .admin()
        .execute_for_tests(&format!(
            "UPDATE scope_requests SET submitted_at_unix = 4 WHERE id = 'req_1';
             UPDATE scope_requests
                SET name = 'login-timeout', head_oid = 'draft-head', pushed_at_unix = 6,
                    base_kind = 'Request', review_base_oid = '{parent_head}',
                    parent_request_id = 'req_1', parent_revision_id = '{pin}',
                    parent_head_oid = '{parent_head}', stack_status = 'waiting_for_parent',
                    stack_depth = 2
              WHERE id = 'req_ask'"
        ))
        .await
        .unwrap();
    let parent = requests.request_by_id("req_1").await.unwrap().unwrap();
    store
        .admin()
        .execute_for_tests(
            "INSERT INTO scope_users VALUES ('user_invited', 'invited', 'invited@scope.test', TRUE);
             INSERT INTO scope_request_invitees VALUES ('req_ask', 'user_invited', 'user_owner', 6)",
        )
        .await
        .unwrap();
    for (viewer, reads_change) in [
        (None, false),
        (Some("user_public"), false),
        (Some("user_invited"), true),
        (Some("user_owner"), true),
    ] {
        let repository = repository_access(store.db.as_ref(), "owner/repo", viewer)
            .await
            .unwrap()
            .unwrap();
        let (_, children) = navigation(store.db.as_ref(), &repository, viewer, &parent, None, 10)
            .await
            .unwrap();
        assert_eq!(
            children
                .requests
                .iter()
                .map(|node| (node.id.as_str(), node.head_oid.as_str()))
                .collect::<Vec<_>>(),
            if reads_change {
                vec![("req_ask", "draft-head")]
            } else {
                Vec::new()
            },
            "{viewer:?}"
        );
        let listed = requests
            .request_list_page(RequestListPageQuery {
                repo_id: "owner/repo",
                viewer_user_id: viewer,
                access: repository.access.clone(),
                views: &repository.views,
                after_id: None,
                name: Some("login-timeout"),
                limit: 10,
            })
            .await
            .unwrap();
        assert_eq!(
            listed
                .iter()
                .map(|row| (
                    row.viewer_reads_change,
                    row.head_oid.as_str(),
                    row.base.parent_request_id(),
                    row.stack_depth
                ))
                .collect::<Vec<_>>(),
            [if reads_change {
                (true, "draft-head", Some("req_1"), 2)
            } else {
                (false, "base", None, 1)
            }],
            "{viewer:?}"
        );
    }

    let mut stacked = crate::db::requests::tests::public_start_input();
    stacked.id = "req_stacked".to_string();
    stacked.name = "on-the-draft".to_string();
    stacked.event_id = "event_stacked".to_string();
    stacked.base = scope_domain::requests::RequestBase::Request {
        request_id: "req_ask".to_string(),
        revision_id: "any".to_string(),
        head_oid: "draft-head".to_string(),
    };
    let error = requests.start_request(stacked).await.unwrap_err();
    assert_eq!(error.message, "parent request not found");

    store
        .admin()
        .execute_for_tests(
            "UPDATE scope_requests
                SET closed_at_unix = 7, closed_by_user_id = 'user_owner',
                    close_reason = 'not_planned', updated_at_unix = 7
              WHERE id = 'req_ask'",
        )
        .await
        .unwrap();
    assert_eq!(
        queued_ids_for(&store, RequestQueueSection::Done, "user_public").await,
        ["req_ask"]
    );

    let mut again = command();
    again.input.id = "req_ask_again".to_string();
    again.input.event_id = "event_asked_again".to_string();
    let error = requests.open_request_from_ask(again).await.unwrap_err();
    assert_eq!(error.message, "this GitHub issue already has a request");
}
