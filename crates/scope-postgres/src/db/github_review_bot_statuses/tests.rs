use super::*;
use crate::db::{
    DiscussionTransition, GitHubImportedComment, GitHubJobOutcome, ImportGitHubReviewCommand,
    MetadataStore, TransitionRequestDiscussionCommand,
    github_pull_requests::tests::{
        OWNER, REPO, claim, connect, oid, push_succeeds, submitted_full_view_request,
    },
    requests::tests::postgres_store,
};
use scope_domain::{
    github_account::{GitHubAccount, GitHubAccountType},
    github_review_bots::GitHubReviewBotInput,
    requests::{GitHubReviewBotState, github_review_comment_client_id},
};

const BOT_ACCOUNT: u64 = 99;

async fn list_bots(store: &MetadataStore, bots: &[(&str, bool)]) {
    store
        .repositories()
        .set_github_review_bots(
            REPO,
            OWNER,
            bots.iter()
                .map(|(login, required)| GitHubReviewBotInput {
                    login: login.to_string(),
                    required: *required,
                })
                .collect(),
        )
        .await
        .unwrap();
}

async fn open_carrier(store: &MetadataStore, request_id: &str, number: u64) {
    push_succeeds(store, request_id, 'a', 10).await;
    let mut carrier = claim(store, "carrier_claim", 10).await.remove(0);
    carrier.record_opened(
        number,
        format!("https://github.com/octo/checks-42/pull/{number}"),
        "main".into(),
        None,
    );
    assert!(
        store
            .requests()
            .finish_github_pull_request(
                &carrier,
                "carrier_claim",
                GitHubJobOutcome::Succeeded(()),
                11
            )
            .await
            .unwrap()
    );
}

async fn due(store: &MetadataStore, at: u64) -> Vec<String> {
    store
        .requests()
        .claim_due_github_review_bot_reconciles(at, at + 100, 10)
        .await
        .unwrap()
        .into_iter()
        .map(|reconcile| reconcile.request_id)
        .collect()
}

fn status(login: &str, state: GitHubReviewBotState) -> GitHubReviewBotStatus {
    GitHubReviewBotStatus {
        login: login.to_string(),
        head_oid: oid('a'),
        state,
    }
}

async fn live_link(store: &MetadataStore) -> String {
    store
        .db
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT link_id FROM scope_github_connections WHERE repo_id = $1",
            [REPO.into()],
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get("", "link_id")
        .unwrap()
}

async fn bot_reviews(store: &MetadataStore) -> RequestBotReviews {
    store
        .requests()
        .request_bot_reviews(REPO, &["req_1".to_string()])
        .await
        .unwrap()
        .remove("req_1")
        .unwrap()
}

#[tokio::test]
async fn open_carriers_of_repositories_with_bots_are_reconciled_until_the_next_due_time() {
    let store = postgres_store();
    connect(&store, true).await;
    submitted_full_view_request(&store, "req_1").await;
    open_carrier(&store, "req_1", 5).await;
    assert!(due(&store, 20).await.is_empty());

    list_bots(&store, &[("greptile-apps[bot]", true)]).await;
    let claimed = store
        .requests()
        .claim_due_github_review_bot_reconciles(20, 120, 10)
        .await
        .unwrap();
    assert_eq!(
        claimed,
        [GitHubReviewBotReconcile {
            request_id: "req_1".into(),
            repo_id: REPO.into(),
            installation_id: 7,
            github_repository_id: 42,
            github_full_name: "octo/checks-42".into(),
            number: 5,
        }]
    );
    assert!(due(&store, 119).await.is_empty());

    let requests = store.requests();
    let working = [status("greptile-apps[bot]", GitHubReviewBotState::Working)];
    assert!(
        requests
            .finish_github_review_bot_reconcile("req_1", Some(&working), 300, 21)
            .await
            .unwrap()
    );
    assert!(
        !requests
            .finish_github_review_bot_reconcile("req_1", Some(&working), 300, 22)
            .await
            .unwrap()
    );
    assert!(due(&store, 299).await.is_empty());
    assert_eq!(bot_reviews(&store).await.statuses, working);

    assert!(
        requests
            .requeue_github_review_bot_reconcile(42, 5)
            .await
            .unwrap()
    );
    assert!(
        !requests
            .requeue_github_review_bot_reconcile(42, 6)
            .await
            .unwrap()
    );
    assert_eq!(due(&store, 23).await, ["req_1"]);
    assert!(
        !requests
            .finish_github_review_bot_reconcile("req_1", None, 500, 24)
            .await
            .unwrap()
    );
    assert!(due(&store, 499).await.is_empty());
    assert_eq!(bot_reviews(&store).await.statuses, working);
}

#[tokio::test]
async fn bot_reviews_carry_the_required_flag_and_the_bot_discussions_left_open() {
    let store = postgres_store();
    connect(&store, true).await;
    submitted_full_view_request(&store, "req_1").await;
    list_bots(
        &store,
        &[
            ("greptile-apps[bot]", false),
            ("chatgpt-codex-connector[bot]", true),
        ],
    )
    .await;
    let reviews = bot_reviews(&store).await;
    assert_eq!(
        reviews
            .bots
            .iter()
            .map(|bot| (bot.login.as_str(), bot.required))
            .collect::<Vec<_>>(),
        [
            ("greptile-apps[bot]", false),
            ("chatgpt-codex-connector[bot]", true)
        ]
    );
    assert!(reviews.open_finding_authors.is_empty());

    let requests = store.requests();
    let imported = requests
        .import_github_review(ImportGitHubReviewCommand {
            request_id: "req_1".into(),
            github_link_id: live_link(&store).await,
            accounts: vec![
                GitHubAccount::new(
                    BOT_ACCOUNT,
                    "greptile-apps[bot]",
                    GitHubAccountType::Bot,
                    None,
                )
                .unwrap(),
            ],
            comments: vec![GitHubImportedComment {
                client_id: github_review_comment_client_id(11),
                reply_to_client_id: None,
                github_account_id: BOT_ACCOUNT,
                source_url: "https://github.com/octo/checks-42/pull/5#discussion_r11".into(),
                body_markdown: "Guard this.".into(),
                anchor: None,
                created_at_unix: 12,
            }],
            now_unix: 12,
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        bot_reviews(&store).await.open_finding_authors,
        ["greptile-apps[bot]"]
    );

    requests
        .transition_request_discussion(TransitionRequestDiscussionCommand {
            request_id: "req_1".into(),
            discussion_id: imported.discussions[0].id.clone(),
            actor_user_id: OWNER.into(),
            event_id: "event_resolve_bot_finding".into(),
            now_unix: 13,
            transition: DiscussionTransition::Resolve,
        })
        .await
        .unwrap();
    assert!(bot_reviews(&store).await.open_finding_authors.is_empty());

    list_bots(&store, &[]).await;
    assert!(
        requests
            .request_bot_reviews(REPO, &["req_1".to_string()])
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn a_head_counts_as_delivered_only_once_its_github_push_succeeded() {
    let store = postgres_store();
    connect(&store, true).await;
    submitted_full_view_request(&store, "req_1").await;
    let requests = store.requests();
    assert_eq!(
        requests
            .github_head_delivered_at("req_1", &oid('a'))
            .await
            .unwrap(),
        None
    );
    push_succeeds(&store, "req_1", 'a', 10).await;
    assert_eq!(
        requests
            .github_head_delivered_at("req_1", &oid('a'))
            .await
            .unwrap(),
        Some(10)
    );
    assert_eq!(
        requests
            .github_head_delivered_at("req_1", &oid('b'))
            .await
            .unwrap(),
        None
    );
}
