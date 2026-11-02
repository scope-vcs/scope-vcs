use super::super::super::fake_github::{workflow_job, workflow_run};
use super::*;
use crate::use_cases::{github_workflow_jobs, github_workflow_runs};
use scope_postgres::db::GitHubCheckCommit;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn webhook_bursts_acknowledge_without_github_and_recover_expired_reads() {
    let request = owner_request("github-webhook-queue", &[REQUIRED_CHECK]).await;
    let (state, fake, head) = (&request.state, &request.fake, request.head());
    push_pass(state, unix_now()).await;
    fake.report_check_runs(
        &head,
        vec![check_run(1, REQUIRED_CHECK, &head, Some("success"))],
    );
    let connection = state
        .metadata
        .repositories()
        .github_connection(TEST_REPO_ID)
        .await
        .unwrap()
        .unwrap();
    github_check_results::refresh_commit_checks(state, &connection, &head)
        .await
        .unwrap();
    let commit = GitHubCheckCommit {
        repo_id: TEST_REPO_ID.into(),
        github_repository_id: GITHUB_REPOSITORY_ID,
        commit_oid: head.clone(),
    };
    let requests = state.metadata.requests();
    assert!(
        requests
            .settled_github_check_read_started_at(&commit)
            .await
            .unwrap()
            .is_some()
    );
    let run = workflow_run(900, &request.branch(), &head, Some("failure"));
    fake.report_workflow_runs(vec![run.clone()]);
    fake.report_workflow_jobs(vec![workflow_job(
        901,
        900,
        "test",
        Some("failure"),
        &["Run tests"],
    )]);
    fake.report_check_runs(
        &head,
        vec![check_run(1, REQUIRED_CHECK, &head, Some("failure"))],
    );
    let before = fake.api_requests.load(Ordering::SeqCst);
    let checks_before = fake.check_run_reads.load(Ordering::SeqCst);
    let now = unix_now();
    for _ in 0..3 {
        for (event, subject) in [
            ("check_run", serde_json::json!({"id": 1, "head_sha": head})),
            ("workflow_run", run.clone()),
            (
                "workflow_job",
                serde_json::json!({"id": 901, "run_id": 900, "run_attempt": 1, "head_sha": head}),
            ),
            (
                "workflow_job",
                serde_json::json!({"id": 902, "run_id": 900, "run_attempt": 1, "head_sha": head}),
            ),
        ] {
            let response = webhook(
                state,
                event,
                serde_json::json!({
                    "action": "completed",
                    "repository": {"id": GITHUB_REPOSITORY_ID},
                    (event): subject,
                }),
                WEBHOOK_SECRET,
            )
            .await;
            assert_eq!(response.status(), StatusCode::NO_CONTENT);
        }
    }
    assert_eq!(fake.api_requests.load(Ordering::SeqCst), before);
    assert_eq!(
        requests
            .settled_github_check_read_started_at(&commit)
            .await
            .unwrap(),
        None
    );
    assert!(
        state
            .wakeups
            .github_check_reads
            .notified()
            .now_or_never()
            .is_some()
    );
    assert!(
        state
            .wakeups
            .github_job_reads
            .notified()
            .now_or_never()
            .is_some()
    );
    let stored = stored_request(state, &request.request_id).await;
    let evidence = requests
        .request_merge_evidence(
            &request.request_id,
            &head,
            stored.base_version,
            stored.base.review_base_oid(),
        )
        .await;
    assert!(evidence.is_err());

    let claimed_at = unix_now().max(now);
    assert!(
        requests
            .claim_github_check_refresh(&commit, claimed_at, claimed_at + 120)
            .await
            .unwrap()
    );
    let repositories = state.metadata.repositories();
    assert_eq!(
        repositories
            .claim_due_github_workflow_run_reads(claimed_at, claimed_at + 120, 20)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        repositories
            .claim_due_github_workflow_job_reads(claimed_at, claimed_at + 120, 20)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        github_check_results::reconcile_github_checks_once(state, claimed_at + 110)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        github_workflow_runs::retry_github_workflow_run_reads_once(state, claimed_at + 110)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        github_workflow_jobs::retry_github_workflow_job_reads_once(state, claimed_at + 110)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        github_check_results::reconcile_github_checks_once(state, claimed_at + 120)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        github_workflow_runs::retry_github_workflow_run_reads_once(state, claimed_at + 120)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        github_workflow_jobs::retry_github_workflow_job_reads_once(state, claimed_at + 120)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        fake.check_run_reads.load(Ordering::SeqCst) - checks_before,
        1
    );
    assert_eq!(fake.job_list_reads.load(Ordering::SeqCst), 1);
    assert_eq!(fake.api_requests.load(Ordering::SeqCst) - before, 3);
    assert_eq!(request.checks().await["outcome"], "failed");
    assert!(
        requests
            .settled_github_check_read_started_at(&commit)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        repositories
            .github_workflow_run(TEST_REPO_ID, GITHUB_REPOSITORY_ID, 900)
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(
        repositories
            .github_workflow_jobs(TEST_REPO_ID, GITHUB_REPOSITORY_ID, 900, 1)
            .await
            .unwrap()
            .len(),
        1
    );
    let mut rerun = workflow_run(900, &request.branch(), &head, Some("success"));
    rerun["run_attempt"] = serde_json::json!(2);
    fake.report_workflow_runs(vec![rerun]);
    let mut rerun_job = workflow_job(903, 900, "test", Some("success"), &["Run tests"]);
    rerun_job["run_attempt"] = serde_json::json!(2);
    fake.report_workflow_jobs(vec![rerun_job]);
    let before = fake.api_requests.load(Ordering::SeqCst);
    let response = webhook(
        state,
        "workflow_job",
        serde_json::json!({
            "repository": {"id": GITHUB_REPOSITORY_ID},
            "workflow_job": {"run_id": 900, "run_attempt": 2},
        }),
        WEBHOOK_SECRET,
    )
    .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(fake.api_requests.load(Ordering::SeqCst), before);
    assert_eq!(
        github_workflow_jobs::retry_github_workflow_job_reads_once(state, unix_now())
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        repositories
            .github_workflow_jobs(TEST_REPO_ID, GITHUB_REPOSITORY_ID, 900, 2)
            .await
            .unwrap()
            .len(),
        1
    );
    let now = unix_now();
    github_workflow_jobs::queue_jobs_for_delivery(state, GITHUB_REPOSITORY_ID, 910, 1)
        .await
        .unwrap();
    assert_eq!(
        github_workflow_jobs::retry_github_workflow_job_reads_once(state, now)
            .await
            .unwrap(),
        0
    );
    fake.report_workflow_runs(vec![workflow_run(
        910,
        &request.branch(),
        &head,
        Some("success"),
    )]);
    fake.report_workflow_jobs(vec![workflow_job(
        911,
        910,
        "test",
        Some("success"),
        &["Run tests"],
    )]);
    assert_eq!(
        github_workflow_jobs::retry_github_workflow_job_reads_once(state, now + 31)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        repositories
            .github_workflow_jobs(TEST_REPO_ID, GITHUB_REPOSITORY_ID, 910, 1)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_job_delivery_for_a_deleted_run_stops_retrying() {
    let request = owner_request("github-deleted-run-jobs", &[REQUIRED_CHECK]).await;
    let (state, head) = (&request.state, request.head());
    let response = webhook(
        state,
        "workflow_job",
        serde_json::json!({
            "action": "completed",
            "repository": {"id": GITHUB_REPOSITORY_ID},
            "workflow_job": {"id": 901, "run_id": 900, "run_attempt": 1, "head_sha": head},
        }),
        WEBHOOK_SECRET,
    )
    .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let repositories = state.metadata.repositories();
    let far_future = unix_now() + 30 * 24 * 60 * 60;
    for read in 0..4 {
        let now = far_future + read * 2 * 60 * 60;
        github_workflow_jobs::retry_github_workflow_job_reads_once(state, now)
            .await
            .unwrap();
    }
    let unread = repositories
        .claim_due_github_workflow_job_reads(far_future * 2, far_future * 2 + 60, 1)
        .await
        .unwrap();
    assert!(unread.is_empty());
}
