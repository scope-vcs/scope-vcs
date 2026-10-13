use super::*;
use axum::http::StatusCode;

#[test]
fn child_start_and_adoption_use_the_parent_pin_and_preserve_existing_upstreams() {
    for adopt in [false, true] {
        let dir = TempDir::new("stack-start");
        create_repo_with_head(dir.path());
        let main = git_stdout(dir.path(), ["rev-parse", "HEAD"]);
        fs::write(dir.path().join("parent.txt"), "parent\n").unwrap();
        run_git(dir.path(), ["add", "."]);
        commit_all(dir.path(), "Parent layer");
        let parent_head = git_stdout(dir.path(), ["rev-parse", "HEAD"]);
        let bare = TempDir::new("stack-start-bare");
        run_git(bare.path(), ["init", "--bare"]);
        run_git(
            dir.path(),
            [
                "push",
                bare.path().to_str().unwrap(),
                &format!("{main}:refs/heads/main"),
                "HEAD:refs/heads/parent",
                "HEAD:refs/heads/child",
            ],
        );
        let (server, bodies) =
            stack_server(&parent_head, &main, bare.path(), ContextResponse::None);
        let transport = BareRepoTransport::new(&server, dir.path(), bare.path());
        run_git(dir.path(), ["switch", "--quiet", "-c", "existing"]);
        run_git(dir.path(), ["update-ref", "refs/remotes/scope/main", &main]);
        run_git(
            dir.path(),
            ["branch", "--set-upstream-to=scope/main", "existing"],
        );
        let mut command = transport.command(&server, dir.path());
        command.args([
            "--json",
            "request",
            "start",
            "child",
            "--base",
            "req_parent",
        ]);
        if adopt {
            command.arg("--current-branch");
        }
        let value = success(command.output().unwrap());
        assert_eq!(value["result"]["base_oid"], parent_head);
        assert_eq!(
            value["result"]["request"]["base"]["revision_id"],
            "rev_parent"
        );
        assert_eq!(git_stdout(dir.path(), ["rev-parse", "HEAD"]), parent_head);
        assert_eq!(
            git_stdout(dir.path(), ["rev-parse", "refs/scope/parents/req_parent"]),
            parent_head
        );
        assert_eq!(
            git_stdout(dir.path(), ["branch", "--show-current"]),
            if adopt { "existing" } else { "child" }
        );
        assert_eq!(
            git_stdout(dir.path(), ["rev-parse", "--abbrev-ref", "@{upstream}"]),
            if adopt { "scope/main" } else { "scope/child" }
        );
        let bodies = bodies.lock().unwrap();
        assert_eq!(bodies[0]["base"]["request_id"], "req_parent");
        assert_eq!(bodies[0]["base"]["head_oid"], parent_head);
        assert_eq!(bodies[0]["view"], "private");
    }
}

#[test]
fn same_head_repin_recovers_after_lost_response_or_definitive_rejection() {
    for mode in [
        ContextResponse::LostResponse,
        ContextResponse::LostResponseAdvanced,
        ContextResponse::RejectedOnce,
    ] {
        let dir = TempDir::new("stack-context");
        create_repo_with_head(dir.path());
        let main = git_stdout(dir.path(), ["rev-parse", "HEAD"]);
        fs::write(dir.path().join("parent.txt"), "parent\n").unwrap();
        run_git(dir.path(), ["add", "."]);
        commit_all(dir.path(), "Parent");
        let parent_head = git_stdout(dir.path(), ["rev-parse", "HEAD"]);
        let bare = TempDir::new("stack-context-bare");
        run_git(bare.path(), ["init", "--bare"]);
        run_git(
            dir.path(),
            [
                "push",
                bare.path().to_str().unwrap(),
                &format!("{main}:refs/heads/main"),
                "HEAD:refs/heads/parent",
                "HEAD:refs/heads/child",
            ],
        );
        let (server, bodies) = stack_server(&parent_head, &main, bare.path(), mode);
        let transport = BareRepoTransport::new(&server, dir.path(), bare.path());
        run_git(dir.path(), ["switch", "--quiet", "-c", "source"]);
        run_git(
            dir.path(),
            ["config", "branch.source.scopeRequestId", "req_source"],
        );
        run_git(dir.path(), ["update-ref", "refs/remotes/scope/main", &main]);
        run_git(
            dir.path(),
            ["branch", "--set-upstream-to=scope/main", "source"],
        );
        let push = |revision: &str| {
            transport
                .command(&server, dir.path())
                .args([
                    "--json",
                    "request",
                    "push",
                    "--request",
                    "req_child",
                    "--base-revision",
                    revision,
                ])
                .output()
                .unwrap()
        };
        let first = push("rev_parent");
        assert_eq!(
            first.status.code(),
            Some(if mode != ContextResponse::RejectedOnce {
                6
            } else {
                5
            })
        );
        if mode == ContextResponse::LostResponseAdvanced {
            fs::write(dir.path().join("later.txt"), "later accepted head\n").unwrap();
            run_git(dir.path(), ["add", "."]);
            commit_all(dir.path(), "Advance child after receipt was lost");
            let advanced_head = git_stdout(dir.path(), ["rev-parse", "HEAD"]);
            run_git(
                dir.path(),
                [
                    "push",
                    bare.path().to_str().unwrap(),
                    "HEAD:refs/heads/child",
                ],
            );
            run_git(dir.path(), ["reset", "--hard", &parent_head]);
            run_git(
                dir.path(),
                ["update-ref", "refs/remotes/scope/child", &advanced_head],
            );
            let recovered = push("rev_parent");
            assert_eq!(recovered.status.code(), Some(5), "{recovered:?}");
            let error: Value = serde_json::from_slice(&recovered.stderr).unwrap();
            assert_eq!(error["code"], "conflict");
            assert_eq!(error["recovery"]["request"]["head_oid"], advanced_head);
            assert_eq!(
                git_stdout(dir.path(), ["rev-parse", "refs/remotes/scope/child"]),
                advanced_head,
                "receipt recovery must not rewind tracking to a historical head"
            );
            assert_eq!(
                git_stdout(bare.path(), ["rev-parse", "refs/heads/child"]),
                advanced_head
            );
            assert_eq!(git_stdout(dir.path(), ["rev-parse", "HEAD"]), parent_head);
            let bodies = bodies.lock().unwrap();
            assert_eq!(bodies.len(), 2);
            assert_eq!(bodies[0], bodies[1]);
            continue;
        }
        let second = success(push(if mode == ContextResponse::LostResponse {
            "rev_parent"
        } else {
            "rev_parent_retry"
        }));
        if mode == ContextResponse::LostResponse {
            assert_eq!(second["result"]["request"]["base"]["kind"], "main");
            let repeated = success(push("rev_parent"));
            assert_eq!(repeated["result"]["request"]["base"]["kind"], "main");
        } else {
            assert_eq!(
                second["result"]["request"]["base"]["revision_id"],
                "rev_parent_retry"
            );
        }
        assert_eq!(
            git_stdout(dir.path(), ["config", "branch.source.scopeRequestId"]),
            "req_source"
        );
        assert_eq!(
            git_stdout(dir.path(), ["branch", "--show-current"]),
            "source"
        );
        assert_eq!(
            git_stdout(dir.path(), ["rev-parse", "--abbrev-ref", "@{upstream}"]),
            "scope/main"
        );
        let bodies = bodies.lock().unwrap();
        assert_eq!(bodies.len(), 2);
        if mode == ContextResponse::LostResponse {
            assert_eq!(bodies[0], bodies[1]);
        } else {
            assert_ne!(bodies[0]["operation_key"], bodies[1]["operation_key"]);
        }
        assert_eq!(bodies[0]["expected_base_version"], 1);
        assert_eq!(bodies[0]["expected_parent_base_version"], 1);
        assert!(
            bodies[0]["operation_key"]
                .as_str()
                .unwrap()
                .starts_with("client_revision-context_")
        );
    }
}

#[test]
fn explicit_existing_pin_push_keeps_base_version_and_targeted_source_tracking() {
    let dir = TempDir::new("stack-manual-repin");
    create_repo_with_head(dir.path());
    let main = git_stdout(dir.path(), ["rev-parse", "HEAD"]);
    fs::write(dir.path().join("parent.txt"), "parent\n").unwrap();
    run_git(dir.path(), ["add", "."]);
    commit_all(dir.path(), "Parent");
    let parent_head = git_stdout(dir.path(), ["rev-parse", "HEAD"]);
    let bare = TempDir::new("stack-manual-repin-bare");
    run_git(bare.path(), ["init", "--bare"]);
    run_git(
        dir.path(),
        [
            "push",
            bare.path().to_str().unwrap(),
            &format!("{main}:refs/heads/main"),
            "HEAD:refs/heads/parent",
            "HEAD:refs/heads/child",
        ],
    );
    let (server, _) = stack_server(&parent_head, &main, bare.path(), ContextResponse::None);
    let transport = BareRepoTransport::new(&server, dir.path(), bare.path());
    transport.capture_request_headers();
    run_git(dir.path(), ["switch", "--quiet", "-c", "source"]);
    run_git(
        dir.path(),
        ["config", "branch.source.scopeRequestId", "req_source"],
    );
    run_git(dir.path(), ["update-ref", "refs/remotes/scope/main", &main]);
    run_git(
        dir.path(),
        ["branch", "--set-upstream-to=scope/main", "source"],
    );
    fs::write(dir.path().join("child.txt"), "child\n").unwrap();
    run_git(dir.path(), ["add", "."]);
    commit_all(dir.path(), "Manually restacked child");
    let head = git_stdout(dir.path(), ["rev-parse", "HEAD"]);
    let headers = dir.path().join("publication-headers");
    let result = transport
        .command(&server, dir.path())
        .env("SCOPE_STACK_HEADERS", &headers)
        .args([
            "--json",
            "request",
            "push",
            "--request",
            "req_child",
            "--base-revision",
            "rev_parent",
        ])
        .output()
        .unwrap();
    let result = success(result);
    assert_eq!(result["result"]["request"]["base_version"], 1);
    let captured = fs::read_to_string(&headers).unwrap();
    assert!(captured.contains("Scope-Request-Base: request:req_parent@rev_parent"));
    assert!(captured.contains("Scope-Request-Base-Version: 1"));
    assert!(captured.contains("Scope-Request-Parent-Base-Version: 1"));
    assert!(captured.contains(&format!("Scope-Request-Parent-Head: {parent_head}")));
    assert!(!captured.contains("fixture-token"));
    assert_eq!(
        git_stdout(bare.path(), ["rev-parse", "refs/heads/child"]),
        head
    );
    assert_eq!(
        git_stdout(bare.path(), ["rev-parse", "refs/heads/parent"]),
        parent_head
    );
    assert_eq!(
        git_stdout(dir.path(), ["config", "branch.source.scopeRequestId"]),
        "req_source"
    );
    assert_eq!(
        git_stdout(dir.path(), ["rev-parse", "--abbrev-ref", "@{upstream}"]),
        "scope/main"
    );
    fs::write(dir.path().join("child.txt"), "second accepted child head\n").unwrap();
    run_git(dir.path(), ["add", "."]);
    commit_all(dir.path(), "Advance the accepted child");
    let second_head = git_stdout(dir.path(), ["rev-parse", "HEAD"]);
    success(
        transport
            .command(&server, dir.path())
            .env("SCOPE_STACK_HEADERS", &headers)
            .args([
                "--json",
                "request",
                "push",
                "--request",
                "req_child",
                "--base-revision",
                "rev_parent",
            ])
            .output()
            .unwrap(),
    );
    assert_eq!(
        git_stdout(dir.path(), ["rev-parse", "refs/remotes/scope/child"]),
        second_head
    );
    run_git(dir.path(), ["reset", "--hard", &head]);
    success(
        transport
            .command(&server, dir.path())
            .env("SCOPE_STACK_HEADERS", &headers)
            .args([
                "--json",
                "request",
                "push",
                "--request",
                "req_child",
                "--base-revision",
                "rev_parent",
            ])
            .output()
            .unwrap(),
    );
    assert_eq!(
        git_stdout(bare.path(), ["rev-parse", "refs/heads/child"]),
        head,
        "a historical matching revision suppressed an intentional rollback"
    );
    run_git(bare.path(), ["update-ref", "refs/heads/child", &main]);
    fs::write(
        dir.path().join("child.txt"),
        "child after concurrent update\n",
    )
    .unwrap();
    run_git(dir.path(), ["add", "."]);
    commit_all(dir.path(), "Child after concurrent update");
    let raced = transport
        .command(&server, dir.path())
        .env("SCOPE_STACK_HEADERS", &headers)
        .args([
            "--json",
            "request",
            "push",
            "--request",
            "req_child",
            "--base-revision",
            "rev_parent",
        ])
        .output()
        .unwrap();
    assert_eq!(raced.status.code(), Some(5), "{raced:?}");
    let error: Value = serde_json::from_slice(&raced.stderr).unwrap();
    assert_eq!(error["code"], "conflict");
    assert_eq!(
        git_stdout(bare.path(), ["rev-parse", "refs/heads/child"]),
        main
    );
}

#[test]
fn same_head_explicit_reparent_publishes_main_or_another_request_without_changing_source_binding() {
    for selector in ["main", "req_other"] {
        let dir = TempDir::new("same-head-reparent");
        create_repo_with_head(dir.path());
        let head = git_stdout(dir.path(), ["rev-parse", "HEAD"]);
        let bare = TempDir::new("same-head-reparent-bare");
        run_git(bare.path(), ["init", "--bare"]);
        run_git(
            dir.path(),
            [
                "push",
                bare.path().to_str().unwrap(),
                "HEAD:refs/heads/main",
                "HEAD:refs/heads/parent",
                "HEAD:refs/heads/other",
                "HEAD:refs/heads/child",
            ],
        );
        let (server, bodies) = stack_server(&head, &head, bare.path(), ContextResponse::None);
        let transport = BareRepoTransport::new(&server, dir.path(), bare.path());
        run_git(dir.path(), ["switch", "-c", "source"]);
        run_git(
            dir.path(),
            ["config", "branch.source.scopeRequestId", "req_source"],
        );
        run_git(dir.path(), ["update-ref", "refs/remotes/scope/main", &head]);
        run_git(
            dir.path(),
            ["branch", "--set-upstream-to=scope/main", "source"],
        );
        let value = success(
            transport
                .command(&server, dir.path())
                .args([
                    "--json",
                    "request",
                    "push",
                    "--request",
                    "req_child",
                    "--base",
                    selector,
                ])
                .output()
                .unwrap(),
        );
        assert_eq!(
            value["result"]["request"]["base"]["kind"],
            if selector == "main" {
                "main"
            } else {
                "request"
            }
        );
        assert_eq!(value["result"]["request"]["base_version"], 2);
        let bodies = bodies.lock().unwrap();
        assert_eq!(bodies.len(), 1);
        if selector == "main" {
            assert_eq!(bodies[0]["base"]["review_base_oid"], head);
            assert!(bodies[0]["expected_parent_base_version"].is_null());
        } else {
            assert_eq!(bodies[0]["base"]["request_id"], "req_other");
            assert_eq!(bodies[0]["expected_parent_base_version"], 1);
        }
        assert_eq!(
            git_stdout(dir.path(), ["config", "branch.source.scopeRequestId"]),
            "req_source"
        );
        assert_eq!(
            git_stdout(dir.path(), ["rev-parse", "--abbrev-ref", "@{upstream}"]),
            "scope/main"
        );
        assert_eq!(
            git_stdout(bare.path(), ["rev-parse", "refs/heads/child"]),
            head
        );
    }
}

#[test]
fn changed_head_explicit_main_publication_sends_only_main_base_headers() {
    let dir = TempDir::new("main-publication-headers");
    create_repo_with_head(dir.path());
    let main = git_stdout(dir.path(), ["rev-parse", "HEAD"]);
    let bare = TempDir::new("main-publication-headers-bare");
    run_git(bare.path(), ["init", "--bare"]);
    run_git(
        dir.path(),
        [
            "push",
            bare.path().to_str().unwrap(),
            "HEAD:refs/heads/main",
            "HEAD:refs/heads/parent",
            "HEAD:refs/heads/child",
        ],
    );
    let (server, _) = stack_server_with_base(
        &main,
        &main,
        bare.path(),
        ContextResponse::None,
        Some(json!({"kind":"main","review_base_oid":main})),
        true,
    );
    let transport = BareRepoTransport::new(&server, dir.path(), bare.path());
    transport.capture_request_headers();
    fs::write(dir.path().join("new.txt"), "main based change\n").unwrap();
    run_git(dir.path(), ["add", "."]);
    commit_all(dir.path(), "New main based head");
    let head = git_stdout(dir.path(), ["rev-parse", "HEAD"]);
    let headers = dir.path().join("headers");
    success(
        transport
            .command(&server, dir.path())
            .env("SCOPE_STACK_HEADERS", &headers)
            .args([
                "--json",
                "request",
                "push",
                "--request",
                "req_child",
                "--base",
                "main",
            ])
            .output()
            .unwrap(),
    );
    let headers = fs::read_to_string(headers).unwrap();
    assert!(headers.contains(&format!("Scope-Request-Base: main:{main}")));
    assert!(headers.contains("Scope-Request-Base-Version: 1"));
    assert!(!headers.contains("Scope-Request-Parent-"));
    assert!(!headers.contains("fixture-token"));
    assert_eq!(
        git_stdout(bare.path(), ["rev-parse", "refs/heads/child"]),
        head
    );
    use std::os::unix::fs::PermissionsExt;
    let hook = bare.path().join("hooks/pre-receive");
    fs::write(
        &hook,
        "#!/bin/sh\nprintf 'opaque publication refusal\\n' >&2\nexit 1\n",
    )
    .unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(dir.path().join("new.txt"), "second main based change\n").unwrap();
    run_git(dir.path(), ["add", "."]);
    commit_all(dir.path(), "Main based publication rejected remotely");
    let attempted = git_stdout(dir.path(), ["rev-parse", "HEAD"]);
    let rejected = transport
        .command(&server, dir.path())
        .env("SCOPE_STACK_HEADERS", dir.path().join("headers"))
        .args([
            "--json",
            "request",
            "push",
            "--request",
            "req_child",
            "--base",
            "main",
        ])
        .output()
        .unwrap();
    assert_eq!(rejected.status.code(), Some(5), "{rejected:?}");
    let error: Value = serde_json::from_slice(&rejected.stderr).unwrap();
    assert_eq!(error["code"], "conflict");
    assert_eq!(error["recovery"]["request"]["head_oid"], head);
    assert_eq!(error["recovery"]["selected_base"]["kind"], "main");
    assert_eq!(error["recovery"]["intended_head_oid"], attempted);
    assert!(
        error["recovery"]["transport_error"]
            .as_str()
            .unwrap()
            .contains("opaque publication refusal")
    );
    assert_eq!(
        git_stdout(bare.path(), ["rev-parse", "refs/heads/child"]),
        head
    );
    assert_eq!(
        git_stdout(dir.path(), ["rev-parse", "refs/remotes/scope/child"]),
        head
    );
    assert_eq!(git_stdout(dir.path(), ["rev-parse", "HEAD"]), attempted);
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum ContextResponse {
    None,
    LostResponse,
    LostResponseAdvanced,
    RejectedOnce,
}

fn stack_server(
    parent_head: &str,
    main: &str,
    bare: &std::path::Path,
    context_response: ContextResponse,
) -> (FixtureServer, Arc<Mutex<Vec<Value>>>) {
    stack_server_with_base(parent_head, main, bare, context_response, None, true)
}

pub(super) fn stack_server_with_base(
    parent_head: &str,
    main: &str,
    bare: &std::path::Path,
    context_response: ContextResponse,
    child_base: Option<Value>,
    parent_open: bool,
) -> (FixtureServer, Arc<Mutex<Vec<Value>>>) {
    let mut parent = request();
    parent["id"] = "req_parent".into();
    parent["name"] = "parent".into();
    parent["state"] = if parent_open { "Open" } else { "Closed" }.into();
    parent["view"] = "private".into();
    parent["head_oid"] = parent_head.into();
    parent["main_ancestor_oid"] = main.into();
    parent["base"]["review_base_oid"] = main.into();
    let pin = json!({"kind":"request","request_id":"req_parent","revision_id":"rev_parent","head_oid":parent_head});
    let mut child = parent.clone();
    child["id"] = "req_child".into();
    child["name"] = "child".into();
    child["state"] = "Draft".into();
    child["base"] = pin.clone();
    child["stack_status"] = "waiting_for_parent".into();
    child["stack_depth"] = 2.into();
    if context_response != ContextResponse::None {
        child["base"]["revision_id"] = "rev_previous".into();
    }
    if let Some(base) = child_base {
        child["base"] = base;
    }
    let mut other = parent.clone();
    other["id"] = "req_other".into();
    other["name"] = "other".into();
    other["state"] = "Open".into();
    let child = Arc::new(Mutex::new(child));
    let bodies = Arc::new(Mutex::new(Vec::new()));
    let parent_revision = Arc::new(Mutex::new("rev_parent".to_string()));
    let parent_revision_read = parent_revision.clone();
    let parent_revision_write = parent_revision.clone();
    let start_bodies = bodies.clone();
    let context_bodies = bodies.clone();
    let start_child = child.clone();
    let show_child = child.clone();
    let context_child = child.clone();
    let history_child = child.clone();
    let accepted = Arc::new(Mutex::new(None::<Value>));
    let accepted_read = accepted.clone();
    let accepted_write = accepted.clone();
    let bare = bare.to_path_buf();
    let show_bare = bare.clone();
    let revisions = Arc::new(Mutex::new(Vec::<Value>::new()));
    let repo = repository_response(
        json!({"access":{"actor":"Owner","view":"private","visibility":"private"}}),
    );
    let app = Router::new()
        .route("/v1/session", get(|| async { Json(support::session_response("usr_test", "owner", "test@example.test")) }))
        .route("/v1/repos/owner/repo", get(move || { let repo=repo.clone(); async move { Json(repo) } }))
        .route("/v1/repos/owner/repo/requests/req_other", get(move || { let other=other.clone(); async move { Json(json!({"request":other})) } }))
        .route("/v1/repos/owner/repo/requests/req_other/changes", get(|| async { Json(json!({"comparison":null,"review_revision_id":"rev_parent","revisions":[],"has_earlier_revisions":false})) }))
        .route("/v1/repos/owner/repo/requests/req_parent", get(move || { let parent=parent.clone(); async move { Json(json!({"request":parent})) } }))
        .route("/v1/repos/owner/repo/requests/req_parent/changes", get(move || { let revision=parent_revision_read.clone(); async move { Json(json!({"comparison":null,"review_revision_id":revision.lock().unwrap().clone(),"revisions":[],"has_earlier_revisions":false})) } }))
        .route("/v1/repos/owner/repo/requests", post(move |Json(body):Json<Value>| { let bodies=start_bodies.clone(); let child=start_child.clone(); async move { bodies.lock().unwrap().push(body); Json(json!({"request":child.lock().unwrap().clone()})) } }))
        .route("/v1/repos/owner/repo/requests/req_child", get(move || { let child=show_child.clone(); let bare=show_bare.clone(); async move {
            let mut request=child.lock().unwrap().clone();
            let head=Command::new("git").arg("--git-dir").arg(bare).args(["rev-parse","refs/heads/child"]).output().unwrap();
            request["head_oid"]=String::from_utf8_lossy(&head.stdout).trim().into();
            Json(json!({"request":request}))
        } }))
        .route("/v1/repos/owner/repo/requests/req_child/changes", get(move |Query(query):Query<HashMap<String,String>>| { let child=history_child.clone(); let accepted=accepted_read.clone(); let bare=bare.clone(); let revisions=revisions.clone(); async move {
            if query.get("revision").is_some_and(|revision| revision == "rev_context") {
                let accepted=accepted.lock().unwrap().clone().unwrap();
                return Json(json!({"comparison":null,"review_revision_id":"rev_context","revisions":[accepted],"has_earlier_revisions":false}));
            }
            let child=child.lock().unwrap().clone();
            let head=Command::new("git").arg("--git-dir").arg(bare).args(["rev-parse","refs/heads/child"]).output().unwrap();
            let head=String::from_utf8_lossy(&head.stdout).trim().to_string();
            let mut revisions=revisions.lock().unwrap();
            if let Some(accepted)=accepted.lock().unwrap().clone() && !revisions.iter().any(|revision| revision["id"]==accepted["id"]) { revisions.push(accepted); }
            if revisions.last().is_none_or(|revision| revision["new_head_oid"] != head) {
                let position=revisions.len()+1;
                let old_head=revisions.last().map(|revision| revision["new_head_oid"].clone());
                revisions.push(json!({"id":format!("rev_child_{position}"),"position":position,"actor":null,"base":child["base"],"base_version":child["base_version"],"old_head_oid":old_head,"new_head_oid":head,"commits":[],"inspection":"Complete","created_at_unix":position}));
            }
            Json(json!({"comparison":null,"review_revision_id":revisions.last().unwrap()["id"],"revisions":revisions.clone(),"has_earlier_revisions":false}))
        } }))
        .route("/v1/repos/owner/repo/requests/req_child/revision-context", post(move |Json(body):Json<Value>| { let bodies=context_bodies.clone(); let child=context_child.clone(); let accepted=accepted_write.clone(); let parent_revision=parent_revision_write.clone(); async move {
            let mut bodies=bodies.lock().unwrap();
            bodies.push(body.clone());
            if context_response == ContextResponse::RejectedOnce && bodies.len()==1 {
                *parent_revision.lock().unwrap()="rev_parent_retry".to_string();
                return (StatusCode::CONFLICT, Json(json!({"code":"conflict","message":"parent changed before publication","retryable":false})));
            }
            let mut child=child.lock().unwrap();
            if accepted.lock().unwrap().is_none() {
                *accepted.lock().unwrap()=Some(json!({"id":"rev_context","position":2,"actor":null,"base":body["base"],"base_version":2,"old_head_oid":body["expected_head_oid"],"new_head_oid":body["expected_head_oid"],"commits":[],"inspection":"Complete","created_at_unix":2}));
                if matches!(context_response, ContextResponse::LostResponse | ContextResponse::LostResponseAdvanced) {
                    child["base"]=json!({"kind":"main","review_base_oid":body["base"]["head_oid"]});
                    child["base_version"]=3.into();
                    child["stack_status"]="main".into();
                    child["stack_depth"]=1.into();
                } else {
                    child["base"]=body["base"].clone();
                    child["base_version"]=2.into();
                }
            }
            if matches!(context_response, ContextResponse::LostResponse | ContextResponse::LostResponseAdvanced) && bodies.len()==1 {
                (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"code":"service_unavailable","message":"response lost after publication","retryable":true})))
            } else {
                (StatusCode::OK, Json(json!({"request":child.clone(),"revision_id":"rev_context"})))
            }
        } }));
    let server = FixtureServer {
        server: TestServer::new(app),
        seen: Arc::new(Mutex::new(Vec::new())),
        edited: Arc::new(Mutex::new(Value::Null)),
        auto_merge: Arc::new(Mutex::new(Value::Null)),
    };
    (server, bodies)
}
