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
    opened.record_commented(oid('a'), 11);
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
    commenting.record_commented(oid('b'), 21);

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
    assert_eq!(
        rereading.review_request,
        Some(GitHubReviewRequest {
            head_oid: oid('b'),
            at_unix: 21
        })
    );
    assert_eq!(rereading.pull_request.unwrap().number, 5);
}

async fn request_branch_push(store: &MetadataStore) -> GitHubPush {
    crate::db::github_pushes::latest_github_push(store.db.as_ref(), "req_1")
        .await
        .unwrap()
        .unwrap()
}

async fn connect_publishing(store: &MetadataStore, publishes_main: bool) {
    connect(store, true).await;
    if publishes_main {
        store
            .repositories()
            .set_github_main_publication(REPO, OWNER, true, 1)
            .await
            .unwrap();
    }
}

async fn merged_with_open_carrier(store: &MetadataStore, publishes_main: bool) {
    connect_publishing(store, publishes_main).await;
    open_carrier_and_merge(store, "req_1", 5, 10).await;
}

async fn open_carrier_and_merge(store: &MetadataStore, id: &str, number: u64, at: u64) {
    let mut request = submitted_full_view_request(store, id).await;
    push_succeeds(store, id, 'a', at).await;
    let mut opened = claim(store, &format!("claim_{id}"), at)
        .await
        .into_iter()
        .find(|carrier| carrier.request_id == id)
        .unwrap();
    opened.record_opened(
        number,
        format!("https://github.com/octo/checks-42/pull/{number}"),
        "main".into(),
        None,
    );
    store
        .requests()
        .finish_github_pull_request(
            &opened,
            &format!("claim_{id}"),
            GitHubJobOutcome::Succeeded(()),
            at + 1,
        )
        .await
        .unwrap();

    request.merged_at_unix = Some(at + 10);
    request.merged_by_user_id = Some(OWNER.into());
    request.merged_head_oid = Some(request.head_oid.clone());
    request.merged_main_oid = Some(oid('m'));
    request.updated_at_unix = at + 10;
    persist_ended(store, &request).await;
}

async fn persist_ended(store: &MetadataStore, request: &Request) {
    let tx = sea_orm::TransactionTrait::begin(store.db.as_ref())
        .await
        .unwrap();
    crate::db::request_lifecycle_effects::persist_lifecycle_mutation(&tx, request, &[])
        .await
        .unwrap();
    tx.commit().await.unwrap();
}

async fn branch_push_of(store: &MetadataStore, request_id: &str) -> Option<GitHubPush> {
    crate::db::github_pushes::latest_github_push(store.db.as_ref(), request_id)
        .await
        .unwrap()
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
async fn ended_requests_whose_branches_github_still_has_are_listed_and_released_by_id() {
    let store = postgres_store();
    merged_with_open_carrier(&store, true).await;
    submitted_full_view_request(&store, "req_open").await;
    push_succeeds(&store, "req_open", 'b', 30).await;
    let mut closed = submitted_full_view_request(&store, "req_closed").await;
    push_succeeds(&store, "req_closed", 'c', 31).await;
    closed.closed_at_unix = Some(32);
    closed.updated_at_unix = 32;
    persist_ended(&store, &closed).await;
    let mut never_pushed = submitted_full_view_request(&store, "req_unpushed").await;
    never_pushed.merged_at_unix = Some(33);
    never_pushed.merged_by_user_id = Some(OWNER.into());
    never_pushed.merged_head_oid = Some(never_pushed.head_oid.clone());
    never_pushed.merged_main_oid = Some(oid('n'));
    never_pushed.updated_at_unix = 33;
    persist_ended(&store, &never_pushed).await;
    assert_eq!(
        branch_push_of(&store, "req_closed")
            .await
            .unwrap()
            .target_oid,
        None
    );
    let mut pushed_elsewhere = submitted_full_view_request(&store, "req_elsewhere").await;
    crate::db::github_pushes::queue_github_push(
        store.db.as_ref(),
        REPO,
        &GitHubBranch::Request("req_elsewhere".into()),
        Some(&oid('e')),
        &GitHubPushDestination {
            installation_id: 7,
            github_repository_id: 43,
            github_full_name: "octo/checks-43".into(),
        },
        34,
    )
    .await
    .unwrap();
    pushed_elsewhere.merged_at_unix = Some(35);
    pushed_elsewhere.merged_by_user_id = Some(OWNER.into());
    pushed_elsewhere.merged_head_oid = Some(pushed_elsewhere.head_oid.clone());
    pushed_elsewhere.merged_main_oid = Some(oid('m'));
    pushed_elsewhere.updated_at_unix = 35;
    crate::db::request_rows::save_request_row(store.db.as_ref(), &pushed_elsewhere)
        .await
        .unwrap();

    let requests = store.requests();
    assert_eq!(
        requests
            .terminal_requests_with_pushed_branches(REPO, 43)
            .await
            .unwrap()
            .iter()
            .map(|request| request.id.as_str())
            .collect::<Vec<_>>(),
        ["req_elsewhere"]
    );
    let listed = requests
        .terminal_requests_with_pushed_branches(REPO, 42)
        .await
        .unwrap();
    assert_eq!(
        listed
            .iter()
            .map(|request| (request.id.as_str(), request.merged_main_oid.clone()))
            .collect::<Vec<_>>(),
        [("req_1", Some(oid('m')))]
    );

    requests
        .release_request_branches(REPO, &["req_1".to_string()], 40)
        .await
        .unwrap();
    assert_eq!(request_branch_push(&store).await.target_oid, None);
    assert!(
        requests
            .terminal_requests_with_pushed_branches(REPO, 42)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        branch_push_of(&store, "req_open").await.unwrap().target_oid,
        Some(oid('b'))
    );
}

#[tokio::test]
async fn one_release_queues_the_deletion_of_every_named_branch() {
    let store = postgres_store();
    connect_publishing(&store, true).await;
    open_carrier_and_merge(&store, "req_1", 5, 10).await;
    open_carrier_and_merge(&store, "req_2", 6, 30).await;
    let requests = store.requests();
    assert_eq!(
        requests
            .terminal_requests_with_pushed_branches(REPO, 42)
            .await
            .unwrap()
            .iter()
            .map(|request| request.id.as_str())
            .collect::<Vec<_>>(),
        ["req_1", "req_2"]
    );

    requests
        .release_request_branches(REPO, &["req_1".to_string(), "req_2".to_string()], 50)
        .await
        .unwrap();
    for id in ["req_1", "req_2"] {
        assert_eq!(
            branch_push_of(&store, id).await.unwrap().target_oid,
            None,
            "{id}"
        );
    }
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
