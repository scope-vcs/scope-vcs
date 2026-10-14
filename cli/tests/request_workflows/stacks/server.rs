use super::*;
use axum::http::StatusCode;

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum ContextResponse {
    None,
    LostResponse,
    LostResponseAdvanced,
    RejectedOnce,
}

pub(super) fn stack_server(
    parent_head: &str,
    main: &str,
    bare: &std::path::Path,
    context_response: ContextResponse,
) -> (FixtureServer, Arc<Mutex<Vec<Value>>>) {
    stack_server_with_base(
        parent_head,
        main,
        bare,
        context_response,
        None,
        true,
        "private",
    )
}

pub(crate) fn stack_server_with_base(
    parent_head: &str,
    main: &str,
    bare: &std::path::Path,
    context_response: ContextResponse,
    child_base: Option<Value>,
    parent_open: bool,
    view: &str,
) -> (FixtureServer, Arc<Mutex<Vec<Value>>>) {
    let mut parent = request();
    parent["id"] = "req_parent".into();
    parent["name"] = "parent".into();
    parent["state"] = if parent_open { "Open" } else { "Closed" }.into();
    parent["view"] = view.into();
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
        json!({"access":{"actor":"Owner","view":"private","visibility":"private"},"views":[
            {"id":"public","name":"Public","includes":[],"readers":"anyone"},
            {"id":"private","name":"Private","includes":"all","readers":"assigned"},
            {"id":"agent","name":"Agent","includes":["public"],"readers":"assigned"}
        ]}),
    );
    let app = Router::new()
        .route("/v1/session", get(|| async { Json(support::session_response("usr_test", "owner", "test@example.test")) }))
        .route("/v1/repos/owner/repo", get(move || { let repo=repo.clone(); async move { Json(repo) } }))
        .route("/v1/repos/owner/repo/requests/req_other", get(move || { let other=other.clone(); async move { Json(json!({"request":other,"stack_navigation":json!({"parent":null,"ancestors":[],"ancestors_unavailable":false,"children":{"requests":[],"next_cursor":null}})})) } }))
        .route("/v1/repos/owner/repo/requests/req_other/changes", get(|| async { Json(json!({"comparison":null,"review_revision_id":"rev_parent","revisions":[],"has_earlier_revisions":false})) }))
        .route("/v1/repos/owner/repo/requests/req_parent", get(move || { let parent=parent.clone(); async move { Json(json!({"request":parent,"stack_navigation":json!({"parent":null,"ancestors":[],"ancestors_unavailable":false,"children":{"requests":[],"next_cursor":null}})})) } }))
        .route("/v1/repos/owner/repo/requests/req_parent/changes", get(move || { let revision=parent_revision_read.clone(); async move { Json(json!({"comparison":null,"review_revision_id":revision.lock().unwrap().clone(),"revisions":[],"has_earlier_revisions":false})) } }))
        .route("/v1/repos/owner/repo/requests", post(move |Json(body):Json<Value>| { let bodies=start_bodies.clone(); let child=start_child.clone(); async move { bodies.lock().unwrap().push(body); Json(json!({"request":child.lock().unwrap().clone()})) } }))
        .route("/v1/repos/owner/repo/requests/req_child", get(move || { let child=show_child.clone(); let bare=show_bare.clone(); async move {
            let mut request=child.lock().unwrap().clone();
            let head=Command::new("git").arg("--git-dir").arg(bare).args(["rev-parse","refs/heads/child"]).output().unwrap();
            request["head_oid"]=String::from_utf8_lossy(&head.stdout).trim().into();
            Json(json!({"request":request,"stack_navigation":json!({"parent":null,"ancestors":[],"ancestors_unavailable":false,"children":{"requests":[],"next_cursor":null}})}))
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

pub(crate) fn require_view_transport(
    transport: &BareRepoTransport,
    server: &FixtureServer,
    view: &str,
) {
    let shim = transport.shim.path().join("git");
    let script = fs::read_to_string(&shim).unwrap();
    let expected = format!("{}/git/{view}/owner/repo", server.server.api_url);
    let script = script.replace(
        r#"if [[ "$arg" == "$SCOPE_TEST_PUBLIC_URL" || "$arg" == "$SCOPE_TEST_PRIVATE_URL" ]]; then"#,
        &format!(r#"if [[ "$arg" == '{expected}' ]]; then"#),
    );
    fs::write(shim, script).unwrap();
}
