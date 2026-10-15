use super::*;
use scope_domain::requests::{
    Request as StackRequest, RequestBase, RequestRevision, RequestStackStatus,
};

use super::requests::start_private_request;

mod auto_merge;
mod carried;
mod navigation;
mod reparent;

#[tokio::test]
async fn request_base_headers_reject_partial_duplicate_and_malformed_publication_before_receiving_git()
 {
    let (state, _source, head) =
        super::push_intent_completion::published_git_fixture("request-base-header-validation")
            .await;
    super::requests::create_owner_request(&state, "req_header_validation", &head).await;
    let valid = [
        (
            "scope-request-base",
            "request:req_parent@revision_parent".to_string(),
        ),
        ("scope-request-base-version", "1".to_string()),
        ("scope-request-parent-base-version", "1".to_string()),
        ("scope-request-parent-head", head.clone()),
    ];
    let mut cases = vec![vec![valid[0].clone()]];
    let mut duplicated = valid.to_vec();
    duplicated.push(valid[0].clone());
    cases.push(duplicated);
    for (index, invalid) in [
        (0, "request:req_parent@revision_parent@extra"),
        (1, "0"),
        (2, "+1"),
        (3, "not-a-commit"),
    ] {
        let mut malformed = valid.to_vec();
        malformed[index].1 = invalid.to_string();
        cases.push(malformed);
    }
    let app = router(state.clone());
    for headers in cases {
        let mut request = Request::builder()
            .method("POST")
            .uri("/git/private/owner/repo/git-receive-pack")
            .header(AUTHORIZATION, bearer_header());
        for (name, value) in headers {
            request = request.header(name, value);
        }
        let response = app
            .clone()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), 16 * 1024).await.unwrap();
        assert!(String::from_utf8_lossy(&body).contains("request"));
    }
    let request = state
        .metadata
        .requests()
        .request_by_id("req_header_validation")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(request.head_oid, head);
    assert_eq!(request.base_version, 1);
    assert!(
        state
            .metadata
            .requests()
            .latest_request_revision(&request.id)
            .await
            .unwrap()
            .is_none()
    );
}

struct PrivateStackFixture {
    state: AppState,
    source: TempGitRepo,
    remote: String,
    _server: TestServer,
    parent_id: String,
    child_id: String,
    parent_pin: String,
    child_head: String,
}

impl PrivateStackFixture {
    async fn create(label: &str) -> Self {
        let (state, source, _) = super::push_intent_completion::published_git_fixture(label).await;
        let (origin, server) = spawn_test_server(&state).await;
        let remote = format!("{origin}/git/private/{TEST_REPO_ID}");
        let parent_id = start_private_request(&state, "stack-parent", None).await;
        commit_file(&source, "parent.txt", "parent layer\n");
        assert_git_push(&source, &remote, "stack-parent", None, &[]);
        submit_private_request(&state, &parent_id).await;
        let parent = stack_request(&state, &parent_id).await;
        let parent_revision = state
            .metadata
            .requests()
            .latest_request_revision(&parent_id)
            .await
            .unwrap()
            .unwrap();
        let base = serde_json::json!({
            "kind": "request", "request_id": parent_id,
            "revision_id": parent_revision.id, "head_oid": parent.head_oid,
        });
        let child_id = start_private_request(&state, "stack-child", Some(base)).await;
        commit_file(&source, "child.txt", "child layer\n");
        assert_git_push(&source, &remote, "stack-child", Some(&parent.head_oid), &[]);
        submit_private_request(&state, &child_id).await;
        let child_head = git_head_oid(&source);
        Self {
            state,
            source,
            remote,
            _server: server,
            parent_id,
            child_id,
            parent_pin: parent.head_oid,
            child_head,
        }
    }

    async fn extend_parent(&self) -> String {
        reset_stack_source(&self.source, &self.parent_pin);
        commit_file(&self.source, "parent-fixup.txt", "parent extension\n");
        assert_git_push(
            &self.source,
            &self.remote,
            "stack-parent",
            Some(&self.parent_pin),
            &[],
        );
        git_head_oid(&self.source)
    }

    async fn latest_child_revision(&self) -> RequestRevision {
        self.state
            .metadata
            .requests()
            .latest_request_revision(&self.child_id)
            .await
            .unwrap()
            .unwrap()
    }
}

async fn stack_request(state: &AppState, request_id: &str) -> StackRequest {
    state
        .metadata
        .requests()
        .request_by_id(request_id)
        .await
        .unwrap()
        .unwrap()
}

async fn submit_private_request(state: &AppState, request_id: &str) {
    let response = api_request(
        router(state.clone()),
        "POST",
        &format!("/v1/repos/owner/repo/requests/{request_id}/submit"),
        Some(&bearer_header()),
        Some("{}"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
}

fn git_push_output(
    source: &FsPath,
    remote: &str,
    branch: &str,
    lease: Option<&str>,
    publication: &[String],
) -> std::process::Output {
    let mut args = vec![
        "-c".to_string(),
        format!(
            "http.{remote}.extraHeader=Authorization: {}",
            bearer_header()
        ),
    ];
    for header in publication {
        args.extend([
            "-c".to_string(),
            format!("http.{remote}.extraHeader={header}"),
        ]);
    }
    args.push("push".to_string());
    if let Some(head) = lease {
        args.push(format!("--force-with-lease=refs/heads/{branch}:{head}"));
    }
    args.extend([remote.to_string(), format!("HEAD:refs/heads/{branch}")]);
    run_git_output(
        Some(source),
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
        "publish stack layer",
    )
    .unwrap()
}

fn assert_git_push(
    source: &FsPath,
    remote: &str,
    branch: &str,
    lease: Option<&str>,
    publication: &[String],
) {
    let output = git_push_output(source, remote, branch, lease, publication);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn assert_child_layer(fixture: &PrivateStackFixture, expected_base: &str) {
    let response = api_request(
        router(fixture.state.clone()),
        "GET",
        &format!("/v1/repos/owner/repo/requests/{}/changes", fixture.child_id),
        Some(&bearer_header()),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let changes = response_json(response).await;
    let comparison = &changes["comparison"];
    assert_eq!(comparison["base_oid"], expected_base);
    let paths = comparison["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file["path"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(paths, ["child.txt"]);
}

async fn merge_stack_request(state: &AppState, request_id: &str, head: &str) -> Response {
    let body = serde_json::json!({ "expected_head_oid": head });
    api_request(
        router(state.clone()),
        "POST",
        &format!("/v1/repos/owner/repo/requests/{request_id}/merge"),
        Some(&bearer_header()),
        Some(&body.to_string()),
    )
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn private_stack_extension_lands_parent_then_child_without_rewriting_child_review() {
    let fixture = PrivateStackFixture::create("private-stack-extension-landing").await;
    let original_revision = fixture.latest_child_revision().await;
    assert_child_layer(&fixture, &fixture.parent_pin).await;
    let blocked = merge_stack_request(&fixture.state, &fixture.child_id, &fixture.child_head).await;
    assert_eq!(blocked.status(), StatusCode::CONFLICT);
    let parent_extension = fixture.extend_parent().await;
    let extended_child = stack_request(&fixture.state, &fixture.child_id).await;
    assert_eq!(
        extended_child.stack_status,
        RequestStackStatus::ParentExtended
    );
    assert_eq!(fixture.latest_child_revision().await, original_revision);
    assert_child_layer(&fixture, &fixture.parent_pin).await;
    let parent_merged =
        merge_stack_request(&fixture.state, &fixture.parent_id, &parent_extension).await;
    assert_eq!(parent_merged.status(), StatusCode::OK);
    let retargeted = stack_request(&fixture.state, &fixture.child_id).await;
    assert_eq!(retargeted.head_oid, fixture.child_head);
    assert_eq!(
        retargeted.base,
        RequestBase::Main {
            review_base_oid: fixture.parent_pin.clone()
        }
    );
    assert_eq!(fixture.latest_child_revision().await, original_revision);
    assert_child_layer(&fixture, &fixture.parent_pin).await;
    let child_merged =
        merge_stack_request(&fixture.state, &fixture.child_id, &fixture.child_head).await;
    assert_eq!(child_merged.status(), StatusCode::OK);
    fetch_stack_main(&fixture);
    for ancestor in [&parent_extension, &fixture.child_head] {
        assert!(
            crate::git::command::git_is_ancestor(
                &fixture.source,
                ancestor,
                "FETCH_HEAD",
                "prove landed layer ancestry"
            )
            .unwrap()
        );
    }
    for (path, contents) in [
        ("parent.txt", "parent layer"),
        ("parent-fixup.txt", "parent extension"),
        ("child.txt", "child layer"),
    ] {
        assert_eq!(
            git_stdout_text(
                &fixture.source,
                &["show", &format!("FETCH_HEAD:{path}")],
                "inspect landed stack"
            )
            .unwrap()
            .trim(),
            contents
        );
    }
}

fn base_publication_headers(
    parent: &StackRequest,
    revision: &RequestRevision,
    child_version: u64,
) -> Vec<String> {
    vec![
        format!("Scope-Request-Base: request:{}@{}", parent.id, revision.id),
        format!("Scope-Request-Base-Version: {child_version}"),
        format!("Scope-Request-Parent-Base-Version: {}", parent.base_version),
        format!("Scope-Request-Parent-Head: {}", parent.head_oid),
    ]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn private_stack_rewrite_requires_current_explicit_pin_and_closed_parent_rejects_push() {
    let fixture = PrivateStackFixture::create("private-stack-rewrite-publication").await;
    let old_child = stack_request(&fixture.state, &fixture.child_id).await;
    let original_revision = fixture.latest_child_revision().await;
    let main_ancestor = old_child.main_ancestor_oid.clone();
    reset_stack_source(&fixture.source, &main_ancestor);
    commit_file(&fixture.source, "parent.txt", "rewritten parent\n");
    let rewritten_parent_head = git_head_oid(&fixture.source);
    assert_git_push(
        &fixture.source,
        &fixture.remote,
        "stack-parent",
        Some(&fixture.parent_pin),
        &[],
    );
    reset_stack_source(&fixture.source, &fixture.child_head);
    commit_file(&fixture.source, "child.txt", "child layer revised\n");
    let rejected = git_push_output(
        &fixture.source,
        &fixture.remote,
        "stack-child",
        Some(&fixture.child_head),
        &[],
    );
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("rewritten"));
    assert_eq!(fixture.latest_child_revision().await, original_revision);
    let parent = stack_request(&fixture.state, &fixture.parent_id).await;
    let parent_revision = fixture
        .state
        .metadata
        .requests()
        .latest_request_revision(&parent.id)
        .await
        .unwrap()
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
            &rewritten_parent_head,
            &fixture.parent_pin,
        ],
        "manually restack child",
    )
    .unwrap();
    let stale_headers =
        base_publication_headers(&parent, &parent_revision, old_child.base_version + 1);
    let stale = git_push_output(
        &fixture.source,
        &fixture.remote,
        "stack-child",
        Some(&fixture.child_head),
        &stale_headers,
    );
    assert!(!stale.status.success());
    assert!(String::from_utf8_lossy(&stale.stderr).contains("context changed"));
    assert_eq!(fixture.latest_child_revision().await, original_revision);
    let headers = base_publication_headers(&parent, &parent_revision, old_child.base_version);
    assert_git_push(
        &fixture.source,
        &fixture.remote,
        "stack-child",
        Some(&fixture.child_head),
        &headers,
    );
    let restacked = stack_request(&fixture.state, &fixture.child_id).await;
    assert_eq!(restacked.base.review_base_oid(), rewritten_parent_head);
    assert_ne!(
        fixture.latest_child_revision().await.id,
        original_revision.id
    );
    assert_child_layer(&fixture, &rewritten_parent_head).await;
    let closed = api_request(
        router(fixture.state.clone()),
        "DELETE",
        &format!("/v1/repos/owner/repo/requests/{}", fixture.parent_id),
        Some(&bearer_header()),
        None,
    )
    .await;
    assert_eq!(closed.status(), StatusCode::OK);
    commit_file(&fixture.source, "child.txt", "closed dependency edit\n");
    let closed_push = git_push_output(
        &fixture.source,
        &fixture.remote,
        "stack-child",
        Some(&restacked.head_oid),
        &[],
    );
    assert!(!closed_push.status.success());
    assert!(String::from_utf8_lossy(&closed_push.stderr).contains("open"));
    let blocked_child = stack_request(&fixture.state, &fixture.child_id).await;
    assert_eq!(blocked_child.head_oid, restacked.head_oid);
    assert_eq!(blocked_child.stack_status, RequestStackStatus::ParentClosed);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn same_head_repin_receipt_replays_once_and_rejects_another_payload() {
    let mut fixture = PrivateStackFixture::create("private-stack-same-head-receipt").await;
    let parent_extension = fixture.extend_parent().await;
    reset_stack_source(&fixture.source, &fixture.child_head);
    run_git(
        Some(&fixture.source),
        &[
            "-c",
            "user.name=Scope Test",
            "-c",
            "user.email=scope-test@example.test",
            "merge",
            "--no-ff",
            &parent_extension,
            "-m",
            "sync parent extension",
        ],
        "sync parent into child",
    )
    .unwrap();
    assert_git_push(
        &fixture.source,
        &fixture.remote,
        "stack-child",
        Some(&fixture.child_head),
        &[],
    );
    let child = stack_request(&fixture.state, &fixture.child_id).await;
    let child_revision = fixture.latest_child_revision().await;
    let parent = stack_request(&fixture.state, &fixture.parent_id).await;
    let parent_revision = fixture
        .state
        .metadata
        .requests()
        .latest_request_revision(&parent.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(child.base.review_base_oid(), fixture.parent_pin);
    let mut body = serde_json::json!({
        "operation_key": "same-head-stack-repin",
        "base": {"kind": "request", "request_id": parent.id, "revision_id": parent_revision.id, "head_oid": parent.head_oid},
        "expected_head_oid": child.head_oid,
        "expected_revision_id": child_revision.id,
        "expected_base_version": child.base_version,
        "expected_parent_base_version": parent.base_version,
    });
    let path = format!(
        "/v1/repos/owner/repo/requests/{}/revision-context",
        fixture.child_id
    );
    let (analytics, recording) = scope_product_analytics::ProductAnalytics::recording();
    fixture.state.product_analytics = analytics;
    let accepted = api_request(
        router(fixture.state.clone()),
        "POST",
        &path,
        Some(&bearer_header()),
        Some(&body.to_string()),
    )
    .await;
    assert_eq!(accepted.status(), StatusCode::OK);
    let accepted = response_json(accepted).await;
    let saved = fixture.latest_child_revision().await;
    assert_ne!(saved.id, child_revision.id);
    assert_eq!(saved.new_head_oid, child.head_oid);
    assert_eq!(saved.base.review_base_oid(), parent_extension);
    let replayed = api_request(
        router(fixture.state.clone()),
        "POST",
        &path,
        Some(&bearer_header()),
        Some(&body.to_string()),
    )
    .await;
    assert_eq!(replayed.status(), StatusCode::OK);
    assert_eq!(
        response_json(replayed).await["revision_id"],
        accepted["revision_id"]
    );
    assert_eq!(fixture.latest_child_revision().await, saved);
    assert_eq!(recording.event_names(), ["request:revision_create"]);
    assert_eq!(
        recording.property(0, "request_id"),
        Some(serde_json::json!(fixture.child_id))
    );
    body["expected_base_version"] = serde_json::json!(child.base_version + 1);
    let incompatible = api_request(
        router(fixture.state.clone()),
        "POST",
        &path,
        Some(&bearer_header()),
        Some(&body.to_string()),
    )
    .await;
    assert_eq!(incompatible.status(), StatusCode::CONFLICT);
    assert_eq!(fixture.latest_child_revision().await, saved);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn landed_parent_conflict_persists_for_reads_until_child_resolution_is_published() {
    let fixture = PrivateStackFixture::create("private-stack-integration-conflict").await;
    commit_file(
        &fixture.source,
        "parent.txt",
        "child modifies parent line\n",
    );
    assert_git_push(
        &fixture.source,
        &fixture.remote,
        "stack-child",
        Some(&fixture.child_head),
        &[],
    );
    let child_head = git_head_oid(&fixture.source);
    let reviewed = fixture.latest_child_revision().await;
    reset_stack_source(&fixture.source, &fixture.parent_pin);
    commit_file(&fixture.source, "parent.txt", "parent modifies same line\n");
    let parent_head = git_head_oid(&fixture.source);
    assert_git_push(
        &fixture.source,
        &fixture.remote,
        "stack-parent",
        Some(&fixture.parent_pin),
        &[],
    );
    let landed = merge_stack_request(&fixture.state, &fixture.parent_id, &parent_head).await;
    assert_eq!(landed.status(), StatusCode::OK);
    let retargeted = stack_request(&fixture.state, &fixture.child_id).await;
    assert_eq!(retargeted.head_oid, child_head);
    assert_eq!(retargeted.base.review_base_oid(), fixture.parent_pin);
    assert_eq!(fixture.latest_child_revision().await, reviewed);
    let blocked = merge_stack_request(&fixture.state, &fixture.child_id, &child_head).await;
    assert_eq!(blocked.status(), StatusCode::CONFLICT);
    assert_eq!(
        stack_request(&fixture.state, &fixture.child_id)
            .await
            .stack_status,
        RequestStackStatus::IntegrationConflict
    );
    for path in [
        format!("/v1/repos/owner/repo/requests/{}", fixture.child_id),
        "/v1/repos/owner/repo/requests".to_string(),
    ] {
        let response = api_request(
            router(fixture.state.clone()),
            "GET",
            &path,
            Some(&bearer_header()),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        let child = if let Some(items) = body["requests"].as_array() {
            items
                .iter()
                .find(|item| item["id"] == fixture.child_id)
                .unwrap()
        } else {
            &body["request"]
        };
        assert_eq!(child["stack_status"], "integration_conflict");
        assert_eq!(child["mergeability"]["status"], "IntegrationConflict");
    }
    reset_stack_source(&fixture.source, &child_head);
    fetch_stack_main(&fixture);
    let conflict = run_git_output(
        Some(&fixture.source),
        &[
            "-c",
            "user.name=Scope Test",
            "-c",
            "user.email=scope-test@example.test",
            "merge",
            "--no-ff",
            "--no-commit",
            "FETCH_HEAD",
        ],
        "merge parent landing into child",
    )
    .unwrap();
    assert!(!conflict.status.success());
    assert!(String::from_utf8_lossy(&conflict.stdout).contains("CONFLICT"));
    fs::write(
        fixture.source.join("parent.txt"),
        "resolved parent and child\n",
    )
    .unwrap();
    run_git(
        Some(&fixture.source),
        &["add", "parent.txt"],
        "stage deliberate stack resolution",
    )
    .unwrap();
    commit_all(&fixture.source, "resolve parent integration conflict");
    assert_git_push(
        &fixture.source,
        &fixture.remote,
        "stack-child",
        Some(&child_head),
        &[],
    );
    let resolved = stack_request(&fixture.state, &fixture.child_id).await;
    assert_eq!(resolved.stack_status, RequestStackStatus::Main);
    assert_ne!(fixture.latest_child_revision().await.id, reviewed.id);
    let merged = merge_stack_request(&fixture.state, &fixture.child_id, &resolved.head_oid).await;
    assert_eq!(merged.status(), StatusCode::OK);
    assert_eq!(
        live_file_content(&fixture.state, "/parent.txt")
            .await
            .as_deref(),
        Some("resolved parent and child\n")
    );
}

fn reset_stack_source(source: &FsPath, head: &str) {
    run_git(
        Some(source),
        &["reset", "--hard", head],
        "restore stack source",
    )
    .unwrap();
}

fn fetch_stack_main(fixture: &PrivateStackFixture) {
    let auth = format!(
        "http.{}.extraHeader=Authorization: {}",
        fixture.remote,
        bearer_header()
    );
    run_git(
        Some(&fixture.source),
        &["-c", &auth, "fetch", &fixture.remote, "refs/heads/main"],
        "fetch landed parent history",
    )
    .unwrap();
}
