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
    for mode in [ContextResponse::LostResponse, ContextResponse::RejectedOnce] {
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
            Some(if mode == ContextResponse::LostResponse {
                6
            } else {
                5
            })
        );
        let second = success(push(if mode == ContextResponse::LostResponse {
            "rev_parent"
        } else {
            "rev_parent_retry"
        }));
        if mode == ContextResponse::LostResponse {
            assert_eq!(second["result"]["request"]["base"]["kind"], "main");
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
    let shim = transport.shim.path().join("git");
    let script = fs::read_to_string(&shim).unwrap();
    let capture = r#"for ((i=0; i<${GIT_CONFIG_COUNT:-0}; i++)); do
  key="GIT_CONFIG_VALUE_$i"
  value="${!key}"
  if [[ "$value" == Scope-Request-* ]]; then
    printf '%s\n' "$value" >> "$SCOPE_STACK_HEADERS"
  fi
done
"#;
    fs::write(
        &shim,
        script.replace(
            "exec \"$SCOPE_TEST_REAL_GIT\"",
            &format!("{capture}exec \"$SCOPE_TEST_REAL_GIT\""),
        ),
    )
    .unwrap();
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

#[derive(Clone, Copy, PartialEq)]
enum ContextResponse {
    None,
    LostResponse,
    RejectedOnce,
}

fn stack_server(
    parent_head: &str,
    main: &str,
    bare: &std::path::Path,
    context_response: ContextResponse,
) -> (FixtureServer, Arc<Mutex<Vec<Value>>>) {
    let mut parent = request();
    parent["id"] = "req_parent".into();
    parent["name"] = "parent".into();
    parent["state"] = "Open".into();
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
    let main_oid = main.to_string();
    let bare = bare.to_path_buf();
    let repo = repository_response(
        json!({"access":{"actor":"Owner","view":"private","visibility":"private"}}),
    );
    let app = Router::new()
        .route("/v1/session", get(|| async { Json(support::session_response("usr_test", "owner", "test@example.test")) }))
        .route("/v1/repos/owner/repo", get(move || { let repo=repo.clone(); async move { Json(repo) } }))
        .route("/v1/repos/owner/repo/requests/req_parent", get(move || { let parent=parent.clone(); async move { Json(json!({"request":parent})) } }))
        .route("/v1/repos/owner/repo/requests/req_parent/changes", get(move || { let revision=parent_revision_read.clone(); async move { Json(json!({"comparison":null,"review_revision_id":revision.lock().unwrap().clone(),"revisions":[],"has_earlier_revisions":false})) } }))
        .route("/v1/repos/owner/repo/requests", post(move |Json(body):Json<Value>| { let bodies=start_bodies.clone(); let child=start_child.clone(); async move { bodies.lock().unwrap().push(body); Json(json!({"request":child.lock().unwrap().clone()})) } }))
        .route("/v1/repos/owner/repo/requests/req_child", get(move || { let child=show_child.clone(); async move { Json(json!({"request":child.lock().unwrap().clone()})) } }))
        .route("/v1/repos/owner/repo/requests/req_child/changes", get(move |Query(query):Query<HashMap<String,String>>| { let child=history_child.clone(); let accepted=accepted_read.clone(); let bare=bare.clone(); async move {
            if query.get("revision").is_some_and(|revision| revision == "rev_context") {
                let accepted=accepted.lock().unwrap().clone().unwrap();
                return Json(json!({"comparison":null,"review_revision_id":"rev_context","revisions":[accepted],"has_earlier_revisions":false}));
            }
            let child=child.lock().unwrap().clone();
            let head=Command::new("git").arg("--git-dir").arg(bare).args(["rev-parse","refs/heads/child"]).output().unwrap();
            let head=String::from_utf8_lossy(&head.stdout).trim().to_string();
            Json(json!({"comparison":null,"review_revision_id":"rev_child","revisions":[{"id":"rev_child","position":1,"actor":null,"base":child["base"],"base_version":child["base_version"],"old_head_oid":null,"new_head_oid":head,"commits":[],"inspection":"Complete","created_at_unix":1}],"has_earlier_revisions":false}))
        } }))
        .route("/v1/repos/owner/repo/requests/req_child/revision-context", post(move |Json(body):Json<Value>| { let bodies=context_bodies.clone(); let child=context_child.clone(); let accepted=accepted_write.clone(); let main_oid=main_oid.clone(); let parent_revision=parent_revision_write.clone(); async move {
            let mut bodies=bodies.lock().unwrap();
            bodies.push(body.clone());
            if context_response == ContextResponse::RejectedOnce && bodies.len()==1 {
                *parent_revision.lock().unwrap()="rev_parent_retry".to_string();
                return (StatusCode::CONFLICT, Json(json!({"code":"conflict","message":"parent changed before publication","retryable":false})));
            }
            let mut child=child.lock().unwrap();
            if accepted.lock().unwrap().is_none() {
                *accepted.lock().unwrap()=Some(json!({"id":"rev_context","position":2,"actor":null,"base":body["base"],"base_version":2,"old_head_oid":body["expected_head_oid"],"new_head_oid":body["expected_head_oid"],"commits":[],"inspection":"Complete","created_at_unix":2}));
                if context_response == ContextResponse::LostResponse {
                    child["base"]=json!({"kind":"main","review_base_oid":main_oid});
                    child["base_version"]=3.into();
                    child["stack_status"]="main".into();
                    child["stack_depth"]=1.into();
                } else {
                    child["base"]=body["base"].clone();
                    child["base_version"]=2.into();
                }
            }
            if context_response == ContextResponse::LostResponse && bodies.len()==1 {
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
