use super::*;
use scope_domain::requests::{
    RequestBase, RequestCheckEvaluation, RequestRevision, RequestStackStatus,
};

struct PublicStack {
    state: AppState,
    fake: Arc<FakeGitHub>,
    source: TempGitRepo,
    remote: String,
    _server: TestServer,
    parent_head: String,
    child_id: String,
    child_head: String,
}

impl PublicStack {
    async fn create(label: &str) -> Self {
        let (state, fake, _owner_source) = private_file_repository(label).await;
        let (source, remote, server, parent_head) = approved_contribution(
            &state,
            &format!("{label}-push"),
            "parent.txt",
            "parent layer\n",
        )
        .await;
        assert_eq!(push_pass(&state, unix_now()).await, 1);
        let parent_revision = state
            .metadata
            .requests()
            .latest_request_revision(REQUEST_ID)
            .await
            .unwrap()
            .unwrap();
        let child_id =
            start_child(&state, "landing-child", &parent_head, &parent_revision.id).await;
        push_change(
            &source,
            &remote,
            "refs/heads/landing-child",
            "child.txt",
            "child layer\n",
            "child layer",
        )
        .unwrap();
        let child_head = git_head_oid(&source);
        expect_json(
            api_request(
                router(state.clone()),
                "POST",
                &format!("/v1/repos/owner/repo/requests/{child_id}/submit"),
                Some(&bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL)),
                Some("{}"),
            )
            .await,
            StatusCode::OK,
        )
        .await;
        Self {
            state,
            fake,
            source,
            remote,
            _server: server,
            parent_head,
            child_id,
            child_head,
        }
    }

    async fn revision(&self) -> RequestRevision {
        self.state
            .metadata
            .requests()
            .latest_request_revision(&self.child_id)
            .await
            .unwrap()
            .unwrap()
    }

    async fn evaluation(&self, request_id: &str, head: &str) -> RequestCheckEvaluation {
        self.state
            .metadata
            .requests()
            .request_check_evaluation(request_id, head)
            .await
            .unwrap()
            .unwrap()
    }

    async fn green(&self, tested: &str, run_id: u64) {
        self.fake.report_check_runs(
            tested,
            vec![check_run(run_id, REQUIRED_CHECK, tested, Some("success"))],
        );
        deliver_check_run(&self.state, GITHUB_REPOSITORY_ID, tested).await;
    }

    fn fetch_main(&self) -> String {
        run_git(
            Some(&self.source),
            &["fetch", &self.remote, "refs/heads/main"],
            "fetch public main",
        )
        .unwrap();
        git_stdout_text(
            &self.source,
            &["rev-parse", "FETCH_HEAD"],
            "read public main",
        )
        .unwrap()
        .trim()
        .to_string()
    }

    async fn comparison(&self, revision: Option<&str>) -> serde_json::Value {
        let query = revision
            .map(|id| format!("?revision={id}"))
            .unwrap_or_default();
        expect_json(
            api_request(
                router(self.state.clone()),
                "GET",
                &format!(
                    "/v1/repos/owner/repo/requests/{}/changes{query}",
                    self.child_id
                ),
                Some(&bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL)),
                None,
            )
            .await,
            StatusCode::OK,
        )
        .await["comparison"]
            .clone()
    }

    async fn approve_child(&self) {
        expect_json(
            api_request(
                router(self.state.clone()),
                "POST",
                &repo_request_checks_approve(TEST_REPO_OWNER, TEST_REPO_NAME, &self.child_id),
                Some(&bearer_header_for(MEMBER_SUBJECT, MEMBER_EMAIL)),
                Some(&reviewed_head_body(&self.state, &self.child_id).await),
            )
            .await,
            StatusCode::OK,
        )
        .await;
    }
}

async fn start_child(state: &AppState, name: &str, parent_head: &str, revision_id: &str) -> String {
    let started = expect_json(api_request(router(state.clone()), "POST", "/v1/repos/owner/repo/requests",
        Some(&bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL)),
        Some(&serde_json::json!({"name": name, "view": "public", "base": {
            "kind": "request", "request_id": REQUEST_ID, "revision_id": revision_id, "head_oid": parent_head
        }}).to_string())).await, StatusCode::OK).await;
    started["request"]["id"].as_str().unwrap().to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exact_public_parent_landing_preserves_child_review_and_worker_renews_approved_checks() {
    let fixture = PublicStack::create("github-public-stack-exact").await;
    let original = fixture.revision().await;
    assert!(
        stored_request(&fixture.state, &fixture.child_id)
            .await
            .git_snapshot
            .is_some()
    );
    let comparison = fixture.comparison(None).await;
    assert_eq!(comparison["base_oid"], fixture.parent_head);
    assert_eq!(
        comparison["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|file| file["path"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["child.txt"]
    );
    let member = bearer_header_for(MEMBER_SUBJECT, MEMBER_EMAIL);
    assert_eq!(
        checks(&fixture.state, &fixture.child_id, &member).await["state"],
        "awaiting-approval"
    );
    assert_eq!(push_pass(&fixture.state, unix_now()).await, 0);
    let child_branch = format!("scope/requests/{}", fixture.child_id);
    assert_eq!(fixture.fake.branch_head(&child_branch), None);
    fixture.approve_child().await;
    assert_eq!(push_pass(&fixture.state, unix_now()).await, 1);
    let old = fixture
        .evaluation(&fixture.child_id, &fixture.child_head)
        .await;
    let github = fixture.fake.repository_path();
    for (path, contents) in [
        ("parent.txt", "parent layer\n"),
        ("child.txt", "child layer\n"),
        (PRIVATE_FILE, PRIVATE_CONTENT),
    ] {
        assert_eq!(file_at(&github, &old.tested_oid, path), contents);
    }
    let parent = fixture.evaluation(REQUEST_ID, &fixture.parent_head).await;
    fixture.green(&old.tested_oid, 1).await;
    let auto_path = scope_api_contract::routes::repo_request_auto_merge(
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        &fixture.child_id,
    );
    let enabled = expect_json(api_request(router(fixture.state.clone()), "POST", &auto_path, Some(&member), Some(&serde_json::json!({"expected_revision_id": original.id, "expected_head_oid": fixture.child_head}).to_string())).await, StatusCode::OK).await;
    assert_eq!(enabled["intent"]["status"], "Active");
    assert_eq!(
        enabled["waiting_reason"],
        "Waiting for the parent request to land"
    );
    assert_eq!(
        merge(&fixture.state, &fixture.child_id).await.status(),
        StatusCode::CONFLICT
    );
    fixture.green(&parent.tested_oid, 2).await;
    expect_json(merge(&fixture.state, REQUEST_ID).await, StatusCode::OK).await;
    let actual_view_main = fixture.fetch_main();
    assert_eq!(actual_view_main, fixture.parent_head);
    let child = stored_request(&fixture.state, &fixture.child_id).await;
    assert_eq!(child.head_oid, fixture.child_head);
    assert_eq!(
        child.base,
        RequestBase::Main {
            review_base_oid: fixture.parent_head.clone()
        }
    );
    assert_eq!(child.main_ancestor_oid, actual_view_main);
    assert_eq!(child.stack_status, RequestStackStatus::Main);
    assert_eq!(fixture.revision().await, original);
    assert_eq!(fixture.comparison(None).await, comparison);
    let mut cursor = Default::default();
    crate::use_cases::request_checks::reconcile_request_checks_once(&fixture.state, &mut cursor)
        .await
        .unwrap();
    let renewed = fixture
        .evaluation(&fixture.child_id, &fixture.child_head)
        .await;
    assert_ne!(renewed.tested_oid, old.tested_oid);
    assert_eq!(renewed.revision_id, original.id);
    assert_eq!(
        renewed.state,
        scope_domain::requests::RequestCheckEvaluationState::Started
    );
    fixture.green(&old.tested_oid, 3).await;
    assert_eq!(
        merge(&fixture.state, &fixture.child_id).await.status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        checks(&fixture.state, &fixture.child_id, &member).await["mergeability"]["status"],
        "ChecksPending"
    );
    assert!(push_pass(&fixture.state, unix_now()).await >= 1);
    assert_eq!(
        fixture.fake.branch_head(&child_branch),
        Some(renewed.tested_oid.clone())
    );
    let pending = expect_json(
        api_request(
            router(fixture.state.clone()),
            "GET",
            &auto_path,
            Some(&member),
            None,
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(pending["intent"]["id"], enabled["intent"]["id"]);
    assert_eq!(pending["intent"]["status"], "Active");
    fixture.green(&renewed.tested_oid, 4).await;
    assert_eq!(
        crate::use_cases::request_auto_merge::reconcile_once(&fixture.state, unix_now())
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        stored_request(&fixture.state, &fixture.child_id)
            .await
            .state(),
        scope_domain::requests::RequestState::Merged
    );
    let fulfilled = expect_json(
        api_request(
            router(fixture.state.clone()),
            "GET",
            &auto_path,
            Some(&member),
            None,
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(fulfilled["intent"]["id"], enabled["intent"]["id"]);
    assert_eq!(fulfilled["intent"]["status"], "Fulfilled");
    assert_eq!(
        live_file_content(&fixture.state, "/parent.txt")
            .await
            .as_deref(),
        Some("parent layer\n")
    );
    assert_eq!(
        live_file_content(&fixture.state, "/child.txt")
            .await
            .as_deref(),
        Some("child layer\n")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn anonymous_readers_page_real_public_stack_children_without_losing_parent_context() {
    let fixture = PublicStack::create("github-public-stack-anonymous-pages").await;
    let parent_revision = fixture
        .state
        .metadata
        .requests()
        .latest_request_revision(REQUEST_ID)
        .await
        .unwrap()
        .unwrap();
    let sibling_id = start_child(
        &fixture.state,
        "landing-sibling",
        &fixture.parent_head,
        &parent_revision.id,
    )
    .await;
    run_git(
        Some(&fixture.source),
        &["push", &fixture.remote, "HEAD:refs/heads/landing-sibling"],
        "publish sibling snapshot",
    )
    .unwrap();
    expect_json(
        api_request(
            router(fixture.state.clone()),
            "POST",
            &format!("/v1/repos/owner/repo/requests/{sibling_id}/submit"),
            Some(&bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL)),
            Some("{}"),
        )
        .await,
        StatusCode::OK,
    )
    .await;
    let first = expect_json(
        api_request(
            router(fixture.state.clone()),
            "GET",
            &format!("/v1/repos/owner/repo/requests/{REQUEST_ID}/children?limit=1"),
            None,
            None,
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(first["requests"].as_array().unwrap().len(), 1);
    let cursor = first["next_cursor"].as_str().unwrap();
    let second = expect_json(
        api_request(
            router(fixture.state.clone()),
            "GET",
            &format!("/v1/repos/owner/repo/requests/{REQUEST_ID}/children?limit=1&cursor={cursor}"),
            None,
            None,
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(second["requests"].as_array().unwrap().len(), 1);
    assert!(second["next_cursor"].is_null());
    let ids = std::collections::BTreeSet::from([
        first["requests"][0]["id"].as_str().unwrap(),
        second["requests"][0]["id"].as_str().unwrap(),
    ]);
    assert_eq!(
        ids,
        std::collections::BTreeSet::from([fixture.child_id.as_str(), sibling_id.as_str()])
    );
    for page in [&first, &second] {
        let row = &page["requests"][0];
        assert_eq!(row["head_oid"], fixture.child_head);
        let detail = expect_json(
            api_request(
                router(fixture.state.clone()),
                "GET",
                &format!(
                    "/v1/repos/owner/repo/requests/{}",
                    row["id"].as_str().unwrap()
                ),
                None,
                None,
            )
            .await,
            StatusCode::OK,
        )
        .await;
        assert_eq!(detail["stack_navigation"]["parent"]["kind"], "available");
        assert_eq!(
            detail["stack_navigation"]["parent"]["request"]["id"],
            REQUEST_ID
        );
        assert_eq!(detail["request"]["base"]["head_oid"], fixture.parent_head);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn extended_public_parent_requires_explicit_main_restack_and_keeps_prior_comparison() {
    let fixture = PublicStack::create("github-public-stack-extended").await;
    let original = fixture.revision().await;
    let comparison = fixture.comparison(None).await;
    run_git(
        Some(&fixture.source),
        &["reset", "--hard", &fixture.parent_head],
        "restore parent",
    )
    .unwrap();
    push_change(
        &fixture.source,
        &fixture.remote,
        REQUEST_REF,
        "parent-extension.txt",
        "parent extension\n",
        "parent extension",
    )
    .unwrap();
    let parent_extension = git_head_oid(&fixture.source);
    expect_json(
        approve(&fixture.state, &parent_extension).await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(push_pass(&fixture.state, unix_now()).await, 1);
    let parent = fixture.evaluation(REQUEST_ID, &parent_extension).await;
    fixture.green(&parent.tested_oid, 1).await;
    expect_json(merge(&fixture.state, REQUEST_ID).await, StatusCode::OK).await;
    let view_main = fixture.fetch_main();
    let child = stored_request(&fixture.state, &fixture.child_id).await;
    assert_eq!(child.stack_status, RequestStackStatus::NeedsMainUpdate);
    assert_eq!(child.head_oid, fixture.child_head);
    assert_eq!(child.base.review_base_oid(), fixture.parent_head);
    assert_eq!(fixture.revision().await, original);
    assert_eq!(fixture.comparison(None).await, comparison);
    assert_eq!(
        merge(&fixture.state, &fixture.child_id).await.status(),
        StatusCode::CONFLICT
    );
    run_git(
        Some(&fixture.source),
        &["reset", "--hard", &fixture.child_head],
        "restore child",
    )
    .unwrap();
    run_git(
        Some(&fixture.source),
        &[
            "-c",
            "user.name=Scope Test",
            "-c",
            "user.email=scope-test@example.test",
            "rebase",
            "--onto",
            &view_main,
            &fixture.parent_head,
        ],
        "restack child onto public main",
    )
    .unwrap();
    let headers = [
        format!("Scope-Request-Base: main:{view_main}"),
        format!("Scope-Request-Base-Version: {}", child.base_version),
    ];
    let mut args = vec![
        "-c".to_string(),
        format!(
            "http.{}.extraHeader=Authorization: {}",
            fixture.remote,
            bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL)
        ),
    ];
    for header in headers {
        args.extend([
            "-c".to_string(),
            format!("http.{}.extraHeader={header}", fixture.remote),
        ]);
    }
    args.extend([
        "push".to_string(),
        format!(
            "--force-with-lease=refs/heads/landing-child:{}",
            fixture.child_head
        ),
        fixture.remote.clone(),
        "HEAD:refs/heads/landing-child".to_string(),
    ]);
    run_git(
        Some(&fixture.source),
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
        "publish explicit public main restack",
    )
    .unwrap();
    let restacked = stored_request(&fixture.state, &fixture.child_id).await;
    assert_eq!(restacked.stack_status, RequestStackStatus::Main);
    assert_eq!(restacked.base.review_base_oid(), view_main);
    assert_ne!(fixture.revision().await.id, original.id);
    assert_eq!(fixture.comparison(Some(&original.id)).await, comparison);
    assert_eq!(
        checks(
            &fixture.state,
            &fixture.child_id,
            &bearer_header_for(MEMBER_SUBJECT, MEMBER_EMAIL)
        )
        .await["state"],
        "awaiting-approval"
    );
    fixture.approve_child().await;
    let child_checks = fixture
        .evaluation(&fixture.child_id, &restacked.head_oid)
        .await;
    assert!(push_pass(&fixture.state, unix_now()).await >= 1);
    assert_eq!(
        fixture
            .fake
            .branch_head(&format!("scope/requests/{}", fixture.child_id)),
        Some(child_checks.tested_oid.clone())
    );
    fixture.green(&child_checks.tested_oid, 2).await;
    expect_json(
        merge(&fixture.state, &fixture.child_id).await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(
        live_file_content(&fixture.state, "/parent-extension.txt")
            .await
            .as_deref(),
        Some("parent extension\n")
    );
    assert_eq!(
        live_file_content(&fixture.state, "/child.txt")
            .await
            .as_deref(),
        Some("child layer\n")
    );
}
