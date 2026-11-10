use super::*;
use crate::db::{GitHubMergeCheckedAgainstMain, GitHubPullRequestFinish, RequestStackChildFacts};
use scope_domain::requests::{GitHubMergeMainCheck, RequestState};

pub(crate) const MAIN: char = 'c';
pub(crate) const SQUASH: char = 'd';

pub(crate) fn squash_seen_in(
    main: char,
    stack_children: Vec<RequestStackChildFacts>,
) -> GitHubMergeCheckedAgainstMain {
    GitHubMergeCheckedAgainstMain {
        main: GitHubMergeMainCheck {
            main_oid: oid(main),
            merge_commit_oid: oid(SQUASH),
            main_contains_merge: true,
        },
        stack_children,
    }
}

pub(crate) async fn finish_merged(
    store: &MetadataStore,
    merged: &GitHubPullRequest,
    claim_token: &str,
    seen_in: char,
) -> Result<GitHubPullRequestFinish, PostgresError> {
    store
        .requests()
        .finish_github_pull_request(
            merged,
            claim_token,
            GitHubJobOutcome::Succeeded(()),
            Some(squash_seen_in(seen_in, Vec::new())),
            12,
        )
        .await
}

async fn main_advanced_by(store: &MetadataStore, user_id_sql: &str) {
    store
        .db
        .execute_unprepared(&format!(
            "UPDATE scope_git_heads SET main_advanced_by_user_id = {user_id_sql}"
        ))
        .await
        .unwrap();
}

pub(crate) fn squash_merged_on_github(
    mut claimed: GitHubPullRequest,
    head_oid: &str,
) -> GitHubPullRequest {
    let url = "https://github.com/octo/checks-42/pull/5".to_string();
    claimed.record_opened(5, url.clone(), "main".into(), None);
    claimed.observe(GitHubPullRequestObservation {
        number: 5,
        url,
        base: "main".into(),
        state: GitHubPullRequestObservedState::Merged(GitHubPullRequestMerge {
            merge_commit_oid: oid(SQUASH),
            head_oid: head_oid.to_string(),
        }),
    });
    claimed
}

async fn request_state(store: &MetadataStore) -> RequestState {
    store
        .requests()
        .request_for_tests("req_1")
        .await
        .unwrap()
        .unwrap()
        .state()
}

#[tokio::test]
async fn a_merge_is_recorded_only_with_its_landing_asks_again_when_main_moved_and_waits_for_a_main_pusher()
 {
    let store = postgres_store();
    connect(&store, true).await;
    let mut request = submitted_full_view_request(&store, "req_1").await;
    request.head_oid = oid('a');
    crate::db::request_rows::save_request_row(store.db.as_ref(), &request)
        .await
        .unwrap();
    store
        .db
        .execute_unprepared(&format!(
            "INSERT INTO scope_git_heads (repo_id, head_oid, push_sequence, change_version,
                frontier_digest, main_advanced_by_user_id)
             VALUES ('{REPO}', '{}', 1, 1, 'frontier', '{OWNER}');
             CREATE FUNCTION reject_landing() RETURNS trigger LANGUAGE plpgsql AS $$
             BEGIN
                 IF NEW.merged_at_unix IS NOT NULL THEN
                     RAISE EXCEPTION 'landing failed';
                 END IF;
                 RETURN NEW;
             END $$;
             CREATE TRIGGER reject_landing BEFORE UPDATE ON scope_requests
                 FOR EACH ROW EXECUTE FUNCTION reject_landing()",
            oid(MAIN)
        ))
        .await
        .unwrap();
    push_succeeds(&store, "req_1", 'a', 10).await;
    let merged = squash_merged_on_github(
        claim(&store, "claim_1", 10).await.remove(0),
        &request.head_oid,
    );

    let error = finish_merged(&store, &merged, "claim_1", MAIN)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("landing failed"), "{error}");
    let unrecorded = carrier(&store, "req_1").await.unwrap();
    assert_eq!(unrecorded.pull_request, None);
    assert_eq!(unrecorded.job_state, GitHubPullRequestJobState::Running);
    store
        .db
        .execute_unprepared("DROP TRIGGER reject_landing ON scope_requests")
        .await
        .unwrap();

    assert_eq!(
        finish_merged(&store, &merged, "claim_1", 'b')
            .await
            .unwrap(),
        GitHubPullRequestFinish::Recorded
    );
    assert_eq!(request_state(&store).await, RequestState::Open);
    let asking_again = claim(&store, "claim_2", 12).await.remove(0);
    assert_eq!(asking_again.pull_request, merged.pull_request);
    assert_eq!(asking_again.attempts, 1);

    main_advanced_by(&store, "NULL").await;
    assert_eq!(
        finish_merged(&store, &asking_again, "claim_2", MAIN)
            .await
            .unwrap(),
        GitHubPullRequestFinish::Recorded
    );
    assert_eq!(request_state(&store).await, RequestState::Open);
    assert!(claim(&store, "claim_3", 12).await.is_empty());

    main_advanced_by(&store, &format!("'{OWNER}'")).await;
    store
        .requests()
        .requeue_github_pull_request(42, "req_1", 12)
        .await
        .unwrap();
    let after_main_push = claim(&store, "claim_4", 12).await.remove(0);
    assert_eq!(
        finish_merged(&store, &after_main_push, "claim_4", MAIN)
            .await
            .unwrap(),
        GitHubPullRequestFinish::RecordedAndLanded
    );
    assert_eq!(request_state(&store).await, RequestState::Merged);
}
