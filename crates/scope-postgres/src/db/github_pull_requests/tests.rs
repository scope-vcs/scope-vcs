use super::*;
use crate::db::{
    MetadataStore, SubmitRequestCommand,
    requests::tests::{postgres_store, public_upload_input},
};
use scope_domain::{
    github_connection::ConnectGitHubRepository,
    requests::{
        GitHubBranch, GitHubPullRequestJobState, GitHubPullRequestObservation,
        GitHubPullRequestState, GitHubPush, RequestActorRole, RequestBase, StartRequestInput,
    },
    views::ViewId,
};

pub(crate) const REPO: &str = "owner/repo";
pub(crate) const OWNER: &str = "user_owner";

pub(crate) fn oid(digit: char) -> String {
    digit.to_string().repeat(40)
}

pub(crate) async fn submitted_full_view_request(store: &MetadataStore, id: &str) -> Request {
    let mut request = draft_full_view_request(store, id).await;
    request.submitted_at_unix = Some(3);
    request.updated_at_unix = 3;
    crate::db::request_rows::save_request_row(store.db.as_ref(), &request)
        .await
        .unwrap();
    request
}

async fn draft_full_view_request(store: &MetadataStore, id: &str) -> Request {
    store
        .requests()
        .start_request(StartRequestInput {
            id: id.to_string(),
            repo_id: REPO.to_string(),
            name: format!("branch-{}", id.replace('_', "-")),
            author_user_id: OWNER.to_string(),
            title: Some("Carry me".to_string()),
            author_role: RequestActorRole::Owner,
            author_view: ViewId::private(),
            view: ViewId::private(),
            base: RequestBase::Main {
                review_base_oid: "base".to_string(),
            },
            main_ancestor_oid: "base".to_string(),
            event_id: format!("event_started_{id}"),
            now_unix: 2,
        })
        .await
        .unwrap()
        .request
}

async fn uploaded_draft(store: &MetadataStore, id: &str) {
    draft_full_view_request(store, id).await;
    let mut upload = public_upload_input();
    upload.request_id = id.to_string();
    upload.actor_user_id = OWNER.to_string();
    store
        .requests()
        .record_working_request_upload(upload)
        .await
        .unwrap();
}

async fn submit(store: &MetadataStore, id: &str, at: u64) {
    store
        .requests()
        .submit_request(SubmitRequestCommand {
            request_id: id.to_string(),
            actor_user_id: OWNER.to_string(),
            event_id: format!("event_submitted_{id}"),
            now_unix: at,
        })
        .await
        .unwrap();
}

pub(crate) async fn connect(store: &MetadataStore, open_pull_requests: bool) {
    connect_to(store, 42, open_pull_requests).await;
}

pub(crate) async fn connect_to(
    store: &MetadataStore,
    github_repository_id: u64,
    open_pull_requests: bool,
) {
    let repositories = store.repositories();
    repositories
        .connect_github_repository(
            ConnectGitHubRepository {
                repository_id: REPO.to_string(),
                installation_id: 7,
                github_repository_id,
                github_full_name: format!("octo/checks-{github_repository_id}"),
                github_private: true,
                acknowledge_public: false,
                run_import_count: 0,
                user_id: OWNER.to_string(),
                now_unix: 1,
            },
            async || Ok::<_, PostgresError>(true),
        )
        .await
        .unwrap();
    repositories
        .set_github_review_bot_pull_requests(REPO, OWNER, open_pull_requests, 1)
        .await
        .unwrap();
}

pub(crate) async fn push_succeeds(store: &MetadataStore, request_id: &str, head: char, at: u64) {
    queue_branch_push(
        store,
        &GitHubBranch::Request(request_id.to_string()),
        head,
        at,
    )
    .await;
    let requests = store.requests();
    let push = requests
        .claim_due_github_pushes("push_claim", at, at + 100, 1)
        .await
        .unwrap()
        .remove(0);
    requests
        .finish_github_push(&push.id, "push_claim", GitHubJobOutcome::Succeeded(()), at)
        .await
        .unwrap()
        .unwrap();
}

async fn queue_branch_push(store: &MetadataStore, branch: &GitHubBranch, head: char, at: u64) {
    let connection = store
        .repositories()
        .github_connection(REPO)
        .await
        .unwrap()
        .unwrap();
    crate::db::github_pushes::queue_github_push(
        store.db.as_ref(),
        REPO,
        branch,
        Some(&oid(head)),
        &GitHubPushDestination::of(&connection),
        at,
    )
    .await
    .unwrap();
}

async fn main_publication_succeeds(store: &MetadataStore, main: char, at: u64) {
    queue_branch_push(store, &GitHubBranch::Main, main, at).await;
    let requests = store.requests();
    let push = requests
        .claim_due_github_pushes("main_claim", at, at + 100, 10)
        .await
        .unwrap()
        .into_iter()
        .find(|push| push.branch == GitHubBranch::Main)
        .unwrap();
    requests
        .finish_github_push(
            &push.id,
            "main_claim",
            GitHubJobOutcome::Succeeded(()),
            at + 1,
        )
        .await
        .unwrap()
        .unwrap();
}

async fn carrier(store: &MetadataStore, request_id: &str) -> Option<GitHubPullRequest> {
    github_pull_request(store.db.as_ref(), request_id)
        .await
        .unwrap()
}

pub(crate) async fn claim(store: &MetadataStore, token: &str, at: u64) -> Vec<GitHubPullRequest> {
    store
        .requests()
        .claim_due_github_pull_requests(token, at, at + 100, 10)
        .await
        .unwrap()
}

fn head_of(carrier: &GitHubPullRequest) -> &str {
    match &carrier.intent {
        GitHubPullRequestIntent::Open { head_oid, .. } => head_oid,
        GitHubPullRequestIntent::Close => panic!("expected an open intent"),
    }
}

#[tokio::test]
async fn a_pushed_head_queues_one_carrier_and_the_newest_intent_wins_over_a_running_claim() {
    let store = postgres_store();
    connect(&store, true).await;
    submitted_full_view_request(&store, "req_1").await;
    push_succeeds(&store, "req_1", 'a', 10).await;

    let mut running = claim(&store, "claim_1", 10).await.remove(0);
    assert_eq!(head_of(&running), oid('a'));
    assert_eq!(running.job_state, GitHubPullRequestJobState::Running);
    assert_eq!(running.attempts, 1);

    push_succeeds(&store, "req_1", 'b', 11).await;
    assert!(claim(&store, "claim_2", 11).await.is_empty());

    running.record_opened(
        5,
        "https://github.com/octo/checks/pull/5".into(),
        "main".into(),
        None,
    );
    assert!(
        store
            .requests()
            .finish_github_pull_request(&running, "claim_1", GitHubJobOutcome::Succeeded(()), 12)
            .await
            .unwrap()
    );
    assert!(
        !store
            .requests()
            .finish_github_pull_request(&running, "claim_1", GitHubJobOutcome::Succeeded(()), 12)
            .await
            .unwrap()
    );

    let next = claim(&store, "claim_3", 12).await.remove(0);
    assert_eq!(head_of(&next), oid('b'));
    assert_eq!(next.attempts, 1);
    assert_eq!(next.pull_request.as_ref().unwrap().number, 5);

    let expired = claim(&store, "claim_4", 12 + 100).await.remove(0);
    assert_eq!(expired.attempts, 2);
}

#[tokio::test]
async fn a_failed_attempt_waits_for_its_retry_and_a_final_failure_keeps_the_error() {
    let store = postgres_store();
    connect(&store, true).await;
    submitted_full_view_request(&store, "req_1").await;
    push_succeeds(&store, "req_1", 'a', 10).await;
    let requests = store.requests();

    let first = claim(&store, "claim_1", 10).await.remove(0);
    requests
        .finish_github_pull_request(
            &first,
            "claim_1",
            GitHubJobOutcome::Failed {
                error: "GitHub was down".into(),
                retry_at_unix: Some(40),
            },
            10,
        )
        .await
        .unwrap();
    assert!(claim(&store, "claim_2", 39).await.is_empty());
    let retried = claim(&store, "claim_3", 40).await.remove(0);
    assert_eq!(retried.last_error.as_deref(), Some("GitHub was down"));

    requests
        .finish_github_pull_request(
            &retried,
            "claim_3",
            GitHubJobOutcome::Failed {
                error: "The Scope GitHub App cannot open pull requests here.".into(),
                retry_at_unix: None,
            },
            41,
        )
        .await
        .unwrap();
    let failed = carrier(&store, "req_1").await.unwrap();
    assert_eq!(failed.job_state, GitHubPullRequestJobState::Failed);
    assert!(claim(&store, "claim_4", 10_000).await.is_empty());
}

#[tokio::test]
async fn only_opted_in_repositories_get_carriers_and_enabling_reaches_pushed_requests() {
    let store = postgres_store();
    connect(&store, false).await;
    submitted_full_view_request(&store, "req_1").await;
    push_succeeds(&store, "req_1", 'a', 10).await;
    assert_eq!(carrier(&store, "req_1").await, None);

    store
        .repositories()
        .set_github_review_bot_pull_requests(REPO, OWNER, true, 20)
        .await
        .unwrap();
    let queued = carrier(&store, "req_1").await.unwrap();
    assert_eq!(head_of(&queued), oid('a'));
    assert_eq!(queued.job_state, GitHubPullRequestJobState::Queued);
}

#[tokio::test]
async fn ending_a_request_turns_its_carrier_intent_into_a_close() {
    let store = postgres_store();
    connect(&store, true).await;
    let mut request = submitted_full_view_request(&store, "req_1").await;
    push_succeeds(&store, "req_1", 'a', 10).await;
    let mut opened = claim(&store, "claim_1", 10).await.remove(0);
    opened.record_opened(
        5,
        "https://github.com/octo/checks/pull/5".into(),
        "main".into(),
        None,
    );
    store
        .requests()
        .finish_github_pull_request(&opened, "claim_1", GitHubJobOutcome::Succeeded(()), 11)
        .await
        .unwrap();

    request.closed_at_unix = Some(20);
    request.updated_at_unix = 20;
    queue_carrier_close(store.db.as_ref(), &request, None)
        .await
        .unwrap();
    let closing = claim(&store, "claim_2", 20).await.remove(0);
    assert_eq!(closing.intent, GitHubPullRequestIntent::Close);
    assert_eq!(
        closing.pull_request.unwrap().state,
        GitHubPullRequestState::Open
    );
}

#[tokio::test]
async fn a_claim_for_the_old_repository_cannot_record_its_pull_request_after_a_reconnect() {
    let store = postgres_store();
    connect(&store, true).await;
    submitted_full_view_request(&store, "req_1").await;
    push_succeeds(&store, "req_1", 'a', 10).await;
    let mut running = claim(&store, "claim_1", 10).await.remove(0);
    assert_eq!(running.destination.github_repository_id, 42);

    store
        .repositories()
        .disconnect_github_repository(REPO, OWNER)
        .await
        .unwrap();
    connect_to(&store, 43, true).await;
    push_succeeds(&store, "req_1", 'b', 11).await;

    running.record_opened(
        5,
        "https://github.com/octo/checks-42/pull/5".into(),
        "main".into(),
        None,
    );
    assert!(
        !store
            .requests()
            .finish_github_pull_request(&running, "claim_1", GitHubJobOutcome::Succeeded(()), 12)
            .await
            .unwrap()
    );
    let reconnected = claim(&store, "claim_2", 12).await.remove(0);
    assert_eq!(reconnected.destination.github_repository_id, 43);
    assert_eq!(reconnected.pull_request, None);
    assert_eq!(head_of(&reconnected), oid('b'));
}

async fn opened_carrier(store: &MetadataStore) {
    connect(store, true).await;
    submitted_full_view_request(store, "req_1").await;
    push_succeeds(store, "req_1", 'a', 10).await;
    let mut opened = claim(store, "claim_1", 10).await.remove(0);
    opened.record_opened(
        5,
        "https://github.com/octo/checks-42/pull/5".into(),
        "main".into(),
        None,
    );
    opened.record_commented(oid('a'));
    store
        .requests()
        .finish_github_pull_request(&opened, "claim_1", GitHubJobOutcome::Succeeded(()), 11)
        .await
        .unwrap();
}

#[tokio::test]
async fn a_delivery_requeues_only_the_carrier_in_that_github_repository() {
    let store = postgres_store();
    opened_carrier(&store).await;
    let requests = store.requests();
    assert!(claim(&store, "claim_2", 20).await.is_empty());

    assert!(
        !requests
            .requeue_github_pull_request(43, "req_1", 20)
            .await
            .unwrap()
    );
    assert!(claim(&store, "claim_2", 20).await.is_empty());
    assert!(
        requests
            .requeue_github_pull_request(42, "req_1", 20)
            .await
            .unwrap()
    );
    let rereading = claim(&store, "claim_3", 20).await.remove(0);
    assert_eq!(rereading.pull_request.unwrap().number, 5);
    assert_eq!(rereading.attempts, 1);
}

#[tokio::test]
async fn a_delivery_during_a_running_claim_runs_the_job_again_after_it() {
    let store = postgres_store();
    opened_carrier(&store).await;
    push_succeeds(&store, "req_1", 'b', 20).await;
    let mut commenting = claim(&store, "claim_2", 20).await.remove(0);
    commenting.record_commented(oid('b'));

    let requests = store.requests();
    assert!(
        requests
            .requeue_github_pull_request(42, "req_1", 21)
            .await
            .unwrap()
    );
    assert!(
        requests
            .finish_github_pull_request(&commenting, "claim_2", GitHubJobOutcome::Succeeded(()), 22)
            .await
            .unwrap()
    );
    let rereading = claim(&store, "claim_3", 22).await.remove(0);
    assert_eq!(rereading.commented_head_oid, Some(oid('b')));
    assert_eq!(rereading.pull_request.unwrap().number, 5);
}

async fn request_branch_push(store: &MetadataStore) -> GitHubPush {
    crate::db::github_pushes::latest_github_push(store.db.as_ref(), "req_1")
        .await
        .unwrap()
        .unwrap()
}

async fn merged_with_open_carrier(store: &MetadataStore, publishes_main: bool) {
    connect(store, true).await;
    if publishes_main {
        store
            .repositories()
            .set_github_main_publication(REPO, OWNER, true, 1)
            .await
            .unwrap();
    }
    let mut request = submitted_full_view_request(store, "req_1").await;
    push_succeeds(store, "req_1", 'a', 10).await;
    let mut opened = claim(store, "claim_1", 10).await.remove(0);
    opened.record_opened(
        5,
        "https://github.com/octo/checks-42/pull/5".into(),
        "main".into(),
        None,
    );
    store
        .requests()
        .finish_github_pull_request(&opened, "claim_1", GitHubJobOutcome::Succeeded(()), 11)
        .await
        .unwrap();

    request.merged_at_unix = Some(20);
    request.merged_by_user_id = Some(OWNER.into());
    request.merged_head_oid = Some(request.head_oid.clone());
    request.merged_main_oid = Some(oid('m'));
    request.updated_at_unix = 20;
    let tx = sea_orm::TransactionTrait::begin(store.db.as_ref())
        .await
        .unwrap();
    crate::db::request_lifecycle_effects::persist_lifecycle_mutation(&tx, &request, &[])
        .await
        .unwrap();
    tx.commit().await.unwrap();
}

#[tokio::test]
async fn a_merge_leaves_carrier_and_branch_to_github_only_when_main_is_published() {
    for publishes_main in [true, false] {
        let store = postgres_store();
        merged_with_open_carrier(&store, publishes_main).await;

        let carrier = carrier(&store, "req_1").await.unwrap();
        if publishes_main {
            assert!(
                matches!(carrier.intent, GitHubPullRequestIntent::Open { .. }),
                "{carrier:?}"
            );
            assert!(claim(&store, "claim_2", 20).await.is_empty());
            assert_eq!(request_branch_push(&store).await.target_oid, Some(oid('a')));

            let requests = store.requests();
            assert!(
                requests
                    .requeue_github_pull_request(42, "req_1", 30)
                    .await
                    .unwrap()
            );
            let mut merged_on_github = claim(&store, "claim_3", 30).await.remove(0);
            merged_on_github.observe(GitHubPullRequestObservation {
                number: 5,
                url: "https://github.com/octo/checks-42/pull/5".into(),
                base: "main".into(),
                open: false,
                closed_by_scope: false,
            });
            assert!(
                requests
                    .finish_github_pull_request(
                        &merged_on_github,
                        "claim_3",
                        GitHubJobOutcome::Succeeded(()),
                        31,
                    )
                    .await
                    .unwrap()
            );
            assert_eq!(request_branch_push(&store).await.target_oid, None);
        } else {
            assert_eq!(carrier.intent, GitHubPullRequestIntent::Close);
            assert_eq!(request_branch_push(&store).await.target_oid, None);
        }
    }
}

#[tokio::test]
async fn a_main_publication_containing_the_merge_releases_the_branch_without_an_observation() {
    let store = postgres_store();
    merged_with_open_carrier(&store, true).await;
    assert_eq!(request_branch_push(&store).await.target_oid, Some(oid('a')));

    main_publication_succeeds(&store, 'x', 15).await;
    assert_eq!(request_branch_push(&store).await.target_oid, Some(oid('a')));

    main_publication_succeeds(&store, 'm', 22).await;
    assert_eq!(request_branch_push(&store).await.target_oid, None);
}

#[tokio::test]
async fn a_request_submitted_after_its_head_reached_github_gets_its_carrier_at_submission() {
    let store = postgres_store();
    connect(&store, true).await;
    uploaded_draft(&store, "req_1").await;
    push_succeeds(&store, "req_1", 'a', 10).await;
    assert_eq!(carrier(&store, "req_1").await, None);
    submit(&store, "req_1", 12).await;
    let queued = carrier(&store, "req_1").await.unwrap();
    assert_eq!(head_of(&queued), oid('a'));
    assert_eq!(queued.job_state, GitHubPullRequestJobState::Queued);

    uploaded_draft(&store, "req_2").await;
    queue_branch_push(&store, &GitHubBranch::Request("req_2".into()), 'b', 13).await;
    submit(&store, "req_2", 14).await;
    assert_eq!(carrier(&store, "req_2").await, None);
    let requests = store.requests();
    let push = requests
        .claim_due_github_pushes("push_claim", 15, 115, 1)
        .await
        .unwrap()
        .remove(0);
    requests
        .finish_github_push(&push.id, "push_claim", GitHubJobOutcome::Succeeded(()), 15)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(head_of(&carrier(&store, "req_2").await.unwrap()), oid('b'));
}
