use super::super::super::fake_github::{suite_check_run, workflow_job, workflow_run};
use super::public::private_file_repository;
use super::public_repositories::make_github_repository_public;
use super::*;
use crate::use_cases::github_job_logs::read_due_github_job_logs_once;
use crate::use_cases::github_workflow_jobs::retry_github_workflow_job_reads_once;
use scope_domain::{
    github_workflow_jobs::GitHubWorkflowJob,
    github_workflow_runs::GitHubWorkflowRun,
    requests::{GitHubCheckConclusion, GitHubCheckStatus},
};

const RUN_ID: u64 = 900;

fn run_path(run_id: u64) -> String {
    format!("/v1/repos/{TEST_REPO_OWNER}/{TEST_REPO_NAME}/github/workflow-runs/{run_id}")
}

async fn open_run(state: &AppState, run_id: u64, bearer: Option<&str>) -> Response {
    api_request(
        router(state.clone()),
        "GET",
        &run_path(run_id),
        bearer,
        None,
    )
    .await
}

async fn job_log(state: &AppState, job_id: u64) -> Response {
    api_request(
        router(state.clone()),
        "GET",
        &format!("{}/jobs/{job_id}/log", run_path(RUN_ID)),
        Some(&bearer_header()),
        None,
    )
    .await
}

async fn deliver(state: &AppState, event: &str, subject: serde_json::Value) {
    let delivery = webhook(
        state,
        event,
        serde_json::json!({
            "action": "in_progress",
            "repository": { "id": GITHUB_REPOSITORY_ID, "full_name": GITHUB_FULL_NAME },
            (event): subject,
        }),
        WEBHOOK_SECRET,
    )
    .await;
    assert_eq!(delivery.status(), StatusCode::NO_CONTENT);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_github_run_opens_on_the_run_page_with_its_jobs_and_finished_logs() {
    let request = owner_request("github-run-page", &[REQUIRED_CHECK]).await;
    let (state, fake, head) = (&request.state, &request.fake, request.head());
    assert_eq!(push_pass(state, unix_now()).await, 1);
    fake.report_workflow_runs(vec![workflow_run(RUN_ID, &request.branch(), &head, None)]);
    deliver(
        state,
        "workflow_run",
        serde_json::json!({ "id": RUN_ID, "head_sha": head }),
    )
    .await;
    fake.report_workflow_jobs(vec![
        workflow_job(901, RUN_ID, "test", None, &["Set up job", "Run tests"]),
        workflow_job(
            902,
            RUN_ID,
            "lint",
            Some("success"),
            &["Set up job", "Lint"],
        ),
    ]);

    fake.report_check_runs(
        &head,
        vec![suite_check_run(901, REQUIRED_CHECK, &head, None, RUN_ID)],
    );
    deliver_check_run(state, GITHUB_REPOSITORY_ID, &head).await;
    assert_eq!(
        request.checks().await["checks"][0]["run"],
        serde_json::json!({ "run_id": RUN_ID.to_string(), "job_id": "901" })
    );

    let mut events = state.repo_events.subscribe(TEST_REPO_ID);
    let opened = expect_json(
        open_run(state, RUN_ID, Some(&bearer_header())).await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(opened["run"]["request_id"], request.request_id);
    assert_eq!(opened["jobs_not_read_yet"], true);
    assert_eq!(opened["jobs"], serde_json::json!([]));
    assert_eq!(fake.job_list_reads.load(Ordering::SeqCst), 0);
    assert_eq!(
        retry_github_workflow_job_reads_once(state, unix_now())
            .await
            .unwrap(),
        1
    );
    assert!(
        std::iter::from_fn(|| events.try_recv().ok()).any(|event| event.kind
            == crate::repo_events::RepoChangeKind::GitHubWorkflowRunChanged {
                github_run_id: RUN_ID
            })
    );
    let opened = expect_json(
        open_run(state, RUN_ID, Some(&bearer_header())).await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(opened["jobs_not_read_yet"], false);
    let jobs = opened["jobs"].as_array().unwrap();
    assert_eq!(
        jobs.iter()
            .map(|job| job["id"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        [901, 902]
    );
    assert_eq!(jobs[0]["status"], "in_progress");
    assert_eq!(jobs[0]["steps"][1]["name"], "Run tests");
    assert_eq!(jobs[0]["steps"][1]["status"], "in_progress");
    expect_json(
        open_run(state, RUN_ID, Some(&bearer_header())).await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(fake.job_list_reads.load(Ordering::SeqCst), 1);

    assert_eq!(job_log(state, 901).await.status(), StatusCode::CONFLICT);

    fake.report_workflow_jobs(vec![workflow_job(
        901,
        RUN_ID,
        "test",
        Some("failure"),
        &["Set up job", "Run tests"],
    )]);
    fake.report_job_log(901, Some("2026-10-05T12:00:59.0000000Z 1 test failed\n"));
    deliver(
        state,
        "workflow_job",
        serde_json::json!({ "id": 901, "run_id": RUN_ID }),
    )
    .await;
    assert_eq!(read_logs(state, 0).await, 1);
    let finished = expect_json(
        open_run(state, RUN_ID, Some(&bearer_header())).await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(finished["jobs"][0]["conclusion"], "failure");
    assert_eq!(finished["jobs"][0]["steps"][1]["status"], "completed");
    assert_eq!(fake.job_list_reads.load(Ordering::SeqCst), 1);

    let repositories = state.metadata.repositories();
    let read = repositories
        .github_workflow_run(TEST_REPO_ID, GITHUB_REPOSITORY_ID, RUN_ID)
        .await
        .unwrap()
        .unwrap()
        .jobs_read;
    assert!(
        repositories
            .replace_github_jobs_read(
                TEST_REPO_ID,
                RUN_ID,
                read,
                Some(scope_domain::github_workflow_jobs::GitHubJobsRead {
                    run_attempt: 1,
                    read_at_unix: unix_now() - 31,
                }),
            )
            .await
            .unwrap()
    );
    let stored = expect_json(
        open_run(state, RUN_ID, Some(&bearer_header())).await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(stored["jobs"][0]["conclusion"], "failure");
    assert_eq!(fake.job_list_reads.load(Ordering::SeqCst), 1);
    assert_eq!(
        retry_github_workflow_job_reads_once(state, unix_now())
            .await
            .unwrap(),
        1
    );
    assert_eq!(fake.job_list_reads.load(Ordering::SeqCst), 2);

    let prefetched = fake.job_log_reads.load(Ordering::SeqCst);
    assert_eq!(
        expect_json(job_log(state, 901).await, StatusCode::OK).await,
        serde_json::json!({ "state": "kept", "text": "2026-10-05T12:00:59.0000000Z 1 test failed\n", "truncated": false })
    );
    assert_eq!(fake.job_log_reads.load(Ordering::SeqCst), prefetched);

    fake.report_job_log(902, Some("2026-10-05T12:00:01.0000000Z lint passed\n"));
    assert_eq!(
        expect_json(job_log(state, 902).await, StatusCode::OK).await,
        serde_json::json!({ "state": "pending" })
    );
    assert_eq!(fake.job_log_reads.load(Ordering::SeqCst), prefetched);
    assert_eq!(read_logs(state, 0).await, 1);
    for _ in 0..2 {
        assert_eq!(
            expect_json(job_log(state, 902).await, StatusCode::OK).await,
            serde_json::json!({ "state": "kept", "text": "2026-10-05T12:00:01.0000000Z lint passed\n", "truncated": false })
        );
    }
    assert_eq!(read_logs(state, 0).await, 0);
    assert_eq!(job_log(state, 903).await.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        open_run(state, 999, Some(&bearer_header())).await.status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_finished_runs_jobs_are_read_again_after_github_could_not_answer() {
    let request = owner_request("github-run-page-retry", &[REQUIRED_CHECK]).await;
    let (state, fake, head) = (&request.state, &request.fake, request.head());
    fake.report_workflow_runs(vec![workflow_run(
        RUN_ID,
        &request.branch(),
        &head,
        Some("success"),
    )]);
    deliver(
        state,
        "workflow_run",
        serde_json::json!({ "id": RUN_ID, "head_sha": head }),
    )
    .await;
    fake.report_workflow_jobs(vec![workflow_job(
        901,
        RUN_ID,
        "test",
        Some("success"),
        &["Run tests"],
    )]);

    fake.job_list_unavailable.store(true, Ordering::SeqCst);
    let unanswered = expect_json(
        open_run(state, RUN_ID, Some(&bearer_header())).await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(unanswered["jobs"], serde_json::json!([]));
    assert_eq!(unanswered["jobs_not_read_yet"], true);
    assert_eq!(fake.job_list_reads.load(Ordering::SeqCst), 0);
    assert_eq!(
        retry_github_workflow_job_reads_once(state, unix_now())
            .await
            .unwrap(),
        0
    );

    fake.job_list_unavailable.store(false, Ordering::SeqCst);
    assert_eq!(
        retry_github_workflow_job_reads_once(state, unix_now() + 31)
            .await
            .unwrap(),
        1
    );
    let answered = expect_json(
        open_run(state, RUN_ID, Some(&bearer_header())).await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(answered["jobs"][0]["id"], 901);
    assert_eq!(fake.job_list_reads.load(Ordering::SeqCst), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_job_without_a_log_is_answered_once_and_never_asked_about_again() {
    let request = owner_request("github-run-page-no-log", &[REQUIRED_CHECK]).await;
    let (state, fake, head) = (&request.state, &request.fake, request.head());
    fake.report_workflow_runs(vec![workflow_run(
        RUN_ID,
        &request.branch(),
        &head,
        Some("success"),
    )]);
    deliver(
        state,
        "workflow_run",
        serde_json::json!({ "id": RUN_ID, "head_sha": head }),
    )
    .await;
    let mut just_finished = workflow_job(906, RUN_ID, "publish", Some("success"), &["Publish"]);
    just_finished["completed_at"] = serde_json::json!(
        time::OffsetDateTime::from_unix_timestamp(unix_now() as i64)
            .unwrap()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap()
    );
    fake.report_workflow_jobs(vec![
        workflow_job(904, RUN_ID, "deploy", Some("skipped"), &[]),
        workflow_job(905, RUN_ID, "test", Some("success"), &["Run tests"]),
        just_finished,
    ]);
    expect_json(
        open_run(state, RUN_ID, Some(&bearer_header())).await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(
        retry_github_workflow_job_reads_once(state, unix_now())
            .await
            .unwrap(),
        1
    );

    for _ in 0..2 {
        assert_eq!(
            expect_json(job_log(state, 904).await, StatusCode::OK).await,
            serde_json::json!({ "state": "not_run" })
        );
    }
    assert_eq!(fake.job_log_reads.load(Ordering::SeqCst), 0);

    for job_id in [905, 906] {
        assert_eq!(
            expect_json(job_log(state, job_id).await, StatusCode::OK).await,
            serde_json::json!({ "state": "pending" })
        );
    }
    assert_eq!(read_logs(state, 0).await, 2);
    for _ in 0..2 {
        assert_eq!(
            expect_json(job_log(state, 905).await, StatusCode::OK).await,
            serde_json::json!({ "state": "expired" })
        );
    }
    assert_eq!(
        expect_json(job_log(state, 906).await, StatusCode::OK).await,
        serde_json::json!({ "state": "pending" })
    );
    assert_eq!(fake.job_log_reads.load(Ordering::SeqCst), 2);

    fake.report_job_log(906, Some("published\n"));
    assert_eq!(read_logs(state, 0).await, 0);
    assert_eq!(read_logs(state, 31).await, 1);
    assert_eq!(
        expect_json(job_log(state, 906).await, StatusCode::OK).await,
        serde_json::json!({ "state": "kept", "text": "published\n", "truncated": false })
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_jobs_long_log_is_read_when_it_finishes_and_only_its_end_is_downloaded() {
    let request = owner_request("github-run-page-long-log", &[REQUIRED_CHECK]).await;
    let (state, fake, head) = (&request.state, &request.fake, request.head());
    fake.report_workflow_runs(vec![workflow_run(
        RUN_ID,
        &request.branch(),
        &head,
        Some("failure"),
    )]);
    deliver(
        state,
        "workflow_run",
        serde_json::json!({ "id": RUN_ID, "head_sha": head }),
    )
    .await;
    let line = "x".repeat(99) + "\n";
    let long = line.repeat(15_000) + "last line\n";
    fake.report_job_log(907, Some(&long));
    fake.report_workflow_jobs(vec![workflow_job(
        907,
        RUN_ID,
        "test",
        Some("failure"),
        &["Run tests"],
    )]);
    deliver(
        state,
        "workflow_job",
        serde_json::json!({ "id": 907, "run_id": RUN_ID }),
    )
    .await;
    assert_eq!(read_logs(state, 0).await, 1);
    assert!(
        fake.job_log_bytes_served.load(Ordering::SeqCst)
            <= scope_domain::github_workflow_jobs::GITHUB_JOB_LOG_LIMIT_BYTES + 1
    );

    let reads = fake.job_log_reads.load(Ordering::SeqCst);
    let response = job_log(state, 907).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    let opened: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(opened["state"], "kept");
    assert_eq!(opened["truncated"], true);
    let text = opened["text"].as_str().unwrap();
    assert!(text.ends_with("last line\n"));
    assert!(text.starts_with(&line));
    assert_eq!(fake.job_log_reads.load(Ordering::SeqCst), reads);
}

async fn read_logs(state: &AppState, seconds_later: u64) -> usize {
    read_due_github_job_logs_once(state, unix_now() + seconds_later)
        .await
        .unwrap()
}

fn stored_run(id: u64, branch: &str) -> GitHubWorkflowRun {
    GitHubWorkflowRun {
        github_run_id: id,
        workflow_name: "ci".into(),
        head_branch: Some(branch.into()),
        head_oid: "a".repeat(40),
        event: "push".into(),
        status: GitHubCheckStatus::Completed,
        conclusion: Some(GitHubCheckConclusion::Success),
        html_url: format!("https://github.com/{GITHUB_FULL_NAME}/actions/runs/{id}"),
        check_suite_id: Some(id),
        run_started_at_unix: Some(10),
        run_attempt: 1,
        updated_at_unix: 20,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_viewers_open_its_github_runs_only_once_the_github_repository_is_public() {
    let (state, fake, _source) = private_file_repository("github-run-page-access").await;
    let repositories = state.metadata.repositories();
    let request_run = stored_run(RUN_ID, &format!("scope/requests/{REQUEST_ID}"));
    let main_run = stored_run(RUN_ID + 1, "main");
    for run in [&request_run, &main_run] {
        repositories
            .save_github_workflow_run(TEST_REPO_ID, GITHUB_REPOSITORY_ID, run)
            .await
            .unwrap();
        repositories
            .save_github_workflow_jobs(
                TEST_REPO_ID,
                GITHUB_REPOSITORY_ID,
                &[GitHubWorkflowJob {
                    github_job_id: run.github_run_id * 10,
                    github_run_id: run.github_run_id,
                    run_attempt: 1,
                    name: "test".into(),
                    status: GitHubCheckStatus::Completed,
                    conclusion: Some(GitHubCheckConclusion::Success),
                    started_at_unix: Some(10),
                    completed_at_unix: Some(20),
                    html_url: String::new(),
                    steps: Vec::new(),
                }],
            )
            .await
            .unwrap();
        repositories
            .replace_github_jobs_read(
                TEST_REPO_ID,
                run.github_run_id,
                None,
                Some(scope_domain::github_workflow_jobs::GitHubJobsRead {
                    run_attempt: 1,
                    read_at_unix: unix_now(),
                }),
            )
            .await
            .unwrap();
    }
    let member = bearer_header_for(MEMBER_SUBJECT, MEMBER_EMAIL);
    let contributor = bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL);

    for run_id in [RUN_ID, RUN_ID + 1] {
        assert_eq!(
            open_run(&state, run_id, Some(&member)).await.status(),
            StatusCode::OK
        );
        assert_eq!(
            open_run(&state, run_id, Some(&contributor)).await.status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            open_run(&state, run_id, None).await.status(),
            StatusCode::NOT_FOUND
        );
    }

    make_github_repository_public(&fake);
    let delivery = webhook(
        &state,
        "repository",
        serde_json::json!({ "action": "publicized", "repository": { "id": GITHUB_REPOSITORY_ID } }),
        WEBHOOK_SECRET,
    )
    .await;
    assert_eq!(delivery.status(), StatusCode::NO_CONTENT);
    let opened = expect_json(
        open_run(&state, RUN_ID, Some(&contributor)).await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(opened["jobs"][0]["id"], RUN_ID * 10);
    assert_eq!(
        open_run(&state, RUN_ID + 1, Some(&contributor))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        open_run(&state, RUN_ID, None).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(fake.job_list_reads.load(Ordering::SeqCst), 0);
}
