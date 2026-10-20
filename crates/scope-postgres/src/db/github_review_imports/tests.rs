use super::*;
use crate::db::{
    MetadataStore, RequestDiscussionsPageQuery, entities,
    requests::tests::{postgres_store, public_start_input, public_upload_input},
};
use scope_domain::{
    github_account::GitHubAccountType,
    requests::{
        RequestActorRole, RequestAttention, RequestAttentionReason, RequestAttentionState,
        RequestDiscussionAuthor, RequestDiscussionResolver, github_review_comment_client_id,
    },
};
use sea_orm::{ActiveModelTrait, EntityTrait, IntoActiveModel};

const REVIEWER: u64 = 900;
const RESOLVER: u64 = 901;

fn account(id: u64, login: &str) -> GitHubAccount {
    GitHubAccount::new(
        id,
        login,
        GitHubAccountType::Bot,
        Some(format!("https://avatars.example/{id}")),
    )
    .unwrap()
}

fn comment(id: u64, reply_to: Option<u64>) -> GitHubImportedComment {
    GitHubImportedComment {
        client_id: github_review_comment_client_id(id),
        reply_to_client_id: reply_to.map(github_review_comment_client_id),
        github_account_id: REVIEWER,
        source_url: format!("https://github.com/octo/repo/pull/1#discussion_r{id}"),
        body_markdown: format!("Comment {id}"),
        anchor: None,
        created_at_unix: id,
    }
}

fn review(comments: Vec<GitHubImportedComment>, now_unix: u64) -> ImportGitHubReviewCommand {
    ImportGitHubReviewCommand {
        request_id: "req_1".to_string(),
        accounts: vec![account(REVIEWER, "greptile-apps[bot]")],
        comments,
        now_unix,
    }
}

async fn open_full_view_request(store: &MetadataStore) -> Request {
    let mut start = public_start_input();
    start.author_user_id = "user_owner".into();
    start.author_role = RequestActorRole::Owner;
    start.view = Views::builtin().full().clone();
    store.requests().start_request(start).await.unwrap();
    let mut upload = public_upload_input();
    upload.actor_user_id = "user_owner".into();
    store
        .requests()
        .record_working_request_upload(upload)
        .await
        .unwrap();
    let mut request = store
        .requests()
        .request_by_id("req_1")
        .await
        .unwrap()
        .unwrap();
    request.submitted_at_unix = Some(3);
    save_request_row(store.db.as_ref(), &request).await.unwrap();
    request
}

async fn discussions(store: &MetadataStore) -> crate::db::RequestDiscussionReadBatch {
    store
        .requests()
        .request_discussions_page(RequestDiscussionsPageQuery {
            request_id: "req_1",
            viewer_user_id: None,
            snapshot_version: u64::MAX >> 1,
            cursor: None,
            discussion_id: None,
            anchor_revision_id: None,
            anchor_commit_oid: None,
            include_revision_anchor: false,
            include_carried_line_anchors: false,
            limit: 50,
        })
        .await
        .unwrap()
}

async fn waiting(store: &MetadataStore, user_id: &str, through: u64) {
    entities::request_attention_state::Model::from_domain(&RequestAttention {
        request_id: "req_1".to_string(),
        user_id: user_id.to_string(),
        state: RequestAttentionState::Waiting,
        reason: RequestAttentionReason::Waiting,
        through_activity_version: through,
        snoozed_until_unix: None,
        updated_at_unix: 3,
        revision: 1,
    })
    .unwrap()
    .into_active_model()
    .insert(store.db.as_ref())
    .await
    .unwrap();
}

async fn attention(store: &MetadataStore, user_id: &str) -> RequestAttention {
    entities::request_attention_state::Entity::find_by_id(("req_1".to_string(), user_id.into()))
        .one(store.db.as_ref())
        .await
        .unwrap()
        .unwrap()
        .try_into_domain()
        .unwrap()
}

#[tokio::test]
async fn a_review_imports_once_with_replies_and_one_author_attention_effect() {
    let store = postgres_store();
    let request = open_full_view_request(&store).await;
    waiting(&store, "user_owner", request.activity_version).await;
    waiting(&store, "user_public", request.activity_version).await;
    let comments = vec![comment(11, None), comment(12, None), comment(13, Some(11))];

    let first = store
        .requests()
        .import_github_review(review(comments.clone(), 10))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.request.activity_version, request.activity_version + 3);
    assert_eq!(first.discussions.len(), 2);
    let author = attention(&store, "user_owner").await;
    assert_eq!(author.state, RequestAttentionState::Active);
    assert_eq!(
        author.through_activity_version,
        first.request.activity_version
    );
    assert_eq!(author.revision, 2);
    assert_eq!(
        attention(&store, "user_public").await.state,
        RequestAttentionState::Waiting
    );

    let again = store
        .requests()
        .import_github_review(review(comments, 20))
        .await
        .unwrap()
        .unwrap();
    assert!(again.discussions.is_empty());
    assert_eq!(
        again.request.activity_version,
        first.request.activity_version
    );
    assert_eq!(attention(&store, "user_owner").await.revision, 2);

    let batch = discussions(&store).await;
    assert_eq!(batch.discussions.len(), 2);
    let root = batch
        .discussions
        .iter()
        .find(|model| model.discussion.client_discussion_id == "github:review-comment:11")
        .unwrap();
    assert_eq!(root.reply_count, 1);
    assert_eq!(
        root.latest_replies[0].reply.author,
        RequestDiscussionAuthor::GitHub {
            github_account_id: REVIEWER,
            source_url: "https://github.com/octo/repo/pull/1#discussion_r13".to_string(),
        }
    );
    assert_eq!(
        batch.actors.github_accounts[&REVIEWER].login,
        "greptile-apps[bot]"
    );
    assert!(batch.actors.users.is_empty());
}

#[tokio::test]
async fn a_reply_without_its_root_becomes_a_discussion_and_later_replies_attach() {
    let store = postgres_store();
    open_full_view_request(&store).await;
    store
        .requests()
        .import_github_review(review(vec![comment(21, Some(20))], 10))
        .await
        .unwrap();
    store
        .requests()
        .import_github_review(review(
            vec![comment(22, Some(21)), comment(23, Some(21))],
            11,
        ))
        .await
        .unwrap();

    let batch = discussions(&store).await;
    assert_eq!(batch.discussions.len(), 1);
    assert_eq!(
        batch.discussions[0].discussion.client_discussion_id,
        "github:review-comment:21"
    );
    assert_eq!(batch.discussions[0].reply_count, 2);
}

#[tokio::test]
async fn thread_resolution_resolves_and_reopens_the_root_discussion() {
    let store = postgres_store();
    open_full_view_request(&store).await;
    let transition = |status| TransitionGitHubDiscussionCommand {
        request_id: "req_1".to_string(),
        client_discussion_id: github_review_comment_client_id(31),
        github_account_id: RESOLVER,
        status,
        now_unix: 40,
    };
    assert_eq!(
        store
            .requests()
            .transition_github_discussion(transition(RequestDiscussionStatus::Resolved))
            .await
            .unwrap(),
        GitHubThreadTransition::MissingDiscussion
    );
    let mut import = review(vec![comment(31, None)], 10);
    import.accounts.push(account(RESOLVER, "octocat"));
    store.requests().import_github_review(import).await.unwrap();

    let GitHubThreadTransition::Applied(resolved) = store
        .requests()
        .transition_github_discussion(transition(RequestDiscussionStatus::Resolved))
        .await
        .unwrap()
    else {
        panic!("resolution must apply");
    };
    let discussion = &resolved.discussions[0];
    assert_eq!(discussion.status, RequestDiscussionStatus::Resolved);
    assert_eq!(
        discussion.resolved_by,
        Some(RequestDiscussionResolver::GitHub(RESOLVER))
    );
    let batch = discussions(&store).await;
    assert_eq!(
        batch.discussions[0].discussion.resolved_by,
        Some(RequestDiscussionResolver::GitHub(RESOLVER))
    );
    assert_eq!(batch.actors.github_accounts[&RESOLVER].login, "octocat");
    assert_eq!(
        store
            .requests()
            .transition_github_discussion(transition(RequestDiscussionStatus::Resolved))
            .await
            .unwrap(),
        GitHubThreadTransition::Unchanged
    );
    let GitHubThreadTransition::Applied(reopened) = store
        .requests()
        .transition_github_discussion(transition(RequestDiscussionStatus::Open))
        .await
        .unwrap()
    else {
        panic!("reopening must apply");
    };
    assert_eq!(reopened.discussions[0].resolved_by, None);
}

#[tokio::test]
async fn draft_requests_import_nothing() {
    let store = postgres_store();
    let mut request = open_full_view_request(&store).await;
    request.submitted_at_unix = None;
    save_request_row(store.db.as_ref(), &request).await.unwrap();
    assert_eq!(
        store
            .requests()
            .import_github_review(review(vec![comment(41, None)], 10))
            .await
            .unwrap(),
        None
    );
    assert!(discussions(&store).await.discussions.is_empty());
}

#[tokio::test]
async fn queued_imports_survive_redelivery_and_threads_keep_their_latest_state() {
    let store = postgres_store();
    open_full_view_request(&store).await;
    let job = |subject| GitHubReviewImportJob {
        request_id: "req_1".to_string(),
        repo_id: "owner/repo".to_string(),
        github_repository_id: 42,
        pull_number: 7,
        subject,
        attempts: 0,
    };
    let review_job = job(GitHubReviewImportSubject::Review { review_id: 5 });
    let thread = |status| {
        job(GitHubReviewImportSubject::Thread {
            root_comment_id: 31,
            status,
            github_account_id: RESOLVER,
        })
    };
    let accounts = [account(RESOLVER, "octocat")];
    let requests = store.requests();
    requests
        .queue_github_review_import(&review_job, &[], 10)
        .await
        .unwrap();
    requests
        .queue_github_review_import(&review_job, &[], 10)
        .await
        .unwrap();
    requests
        .queue_github_review_import(&thread(RequestDiscussionStatus::Resolved), &accounts, 10)
        .await
        .unwrap();
    let mut other_repo = job(GitHubReviewImportSubject::Reply { comment_id: 9 });
    other_repo.repo_id = "other/repo".to_string();
    requests
        .queue_github_review_import(&other_repo, &[], 10)
        .await
        .unwrap();

    let claimed = requests
        .claim_due_github_review_imports(10, 100, 10)
        .await
        .unwrap();
    assert_eq!(claimed.len(), 2);
    let claimed_thread = claimed
        .iter()
        .find(|job| matches!(job.subject, GitHubReviewImportSubject::Thread { .. }))
        .unwrap()
        .clone();
    assert_eq!(claimed_thread.attempts, 1);
    requests
        .queue_github_review_import(&thread(RequestDiscussionStatus::Open), &accounts, 20)
        .await
        .unwrap();
    requests
        .finish_github_review_import(&claimed_thread, None)
        .await
        .unwrap();
    let claimed_review = claimed
        .iter()
        .find(|job| job.subject == review_job.subject)
        .unwrap();
    requests
        .finish_github_review_import(claimed_review, None)
        .await
        .unwrap();

    let remaining = requests
        .claim_due_github_review_imports(20, 200, 10)
        .await
        .unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(
        remaining[0].subject,
        GitHubReviewImportSubject::Thread {
            root_comment_id: 31,
            status: RequestDiscussionStatus::Open,
            github_account_id: RESOLVER,
        }
    );
    assert_eq!(remaining[0].attempts, 1);
}
