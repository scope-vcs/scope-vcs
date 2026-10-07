use super::custom_views::{fixture, member_bearer, member_id, view};
use super::custom_views_requests::{AgentCheckout, agent_checkout, contains_commit};
use super::*;
use scope_domain::requests::{
    MainPushRequestOutcome, Request, RequestRevisionGitFacts, RequestState,
    StartMainPushRequestInput, main_push_request_names,
};

async fn grant_push(state: &AppState) {
    let granted = api_request(
        router(state.clone()),
        "PATCH",
        &format!("/v1/repos/owner/repo/members/{}", member_id()),
        Some(&bearer_header()),
        Some(r#"{"permissions":{"can_push":true,"can_change_file_visibility":false,"view":"agent"}}"#),
    )
    .await;
    expect_json(granted, StatusCode::OK).await;
}

async fn push_main(state: &AppState, checkout: &AgentCheckout, head: &str) {
    let token = view_push_intent(state, head).await;
    let pushed = checkout.push(
        "refs/heads/main",
        &[format!("X-Scope-Push-Intent: {token}")],
    );
    assert!(
        pushed.status.success(),
        "{}",
        String::from_utf8_lossy(&pushed.stderr)
    );
}

async fn view_push_intent(state: &AppState, head: &str) -> String {
    let intent = expect_json(
        api_request(
            router(state.clone()),
            "POST",
            "/v1/repos/owner/repo/push-intents",
            Some(&member_bearer()),
            Some(&serde_json::json!({ "head_oid": head, "view": "agent" }).to_string()),
        )
        .await,
        StatusCode::OK,
    )
    .await;
    intent["token"].as_str().unwrap().to_string()
}

async fn main_push_requests(state: &AppState) -> Vec<Request> {
    let mut requests = state
        .metadata
        .requests()
        .requests_by_repo_id(TEST_REPO_ID)
        .await
        .unwrap()
        .into_iter()
        .filter(|request| request.name.starts_with("main-push-"))
        .collect::<Vec<_>>();
    requests.sort_by(|left, right| left.name.cmp(&right.name));
    requests
}

fn concurrent_push(
    landed: &Request,
    id: &str,
    name: &str,
    base_main_oid: &str,
) -> StartMainPushRequestInput {
    StartMainPushRequestInput {
        id: id.to_string(),
        repo_id: TEST_REPO_ID.to_string(),
        repository_incarnation_id: String::new(),
        pusher_user_id: member_id(),
        pusher_handle: "agent-member".to_string(),
        validated_view: view("agent"),
        name: name.to_string(),
        base_main_oid: base_main_oid.to_string(),
        head_oid: landed.head_oid.clone(),
        git_snapshot: landed.git_snapshot.clone().unwrap(),
        git_facts: RequestRevisionGitFacts {
            contains_old_head: true,
            contained_main_oid: Some(base_main_oid.to_string()),
            contained_main_descends_from_base: true,
        },
        started_event_id: format!("{id}_started"),
        revision_event_id: format!("{id}_revision"),
        submitted_event_id: format!("{id}_submitted"),
        auto_merge_intent_id: format!("{id}_auto_merge"),
        auto_merge_event_id: format!("{id}_auto_merge_event"),
        now_unix: unix_now(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_main_push_through_the_agent_view_lands_as_an_auto_merged_request() {
    let (state, _head, _source) = fixture("agent-main-push").await;
    grant_push(&state).await;
    let checkout = agent_checkout(&state, "agent-main-push-clone").await;
    let agent_main = checkout.fetch_main();
    let head = checkout.commit(&[("src/main.rs", "fn main() { run() }\n")], "agent main");

    let intent = expect_json(
        api_request(
            router(state.clone()),
            "POST",
            "/v1/repos/owner/repo/push-intents",
            Some(&member_bearer()),
            Some(&serde_json::json!({ "head_oid": head, "view": "agent" }).to_string()),
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(intent["lands_as_request"], true);
    assert_eq!(intent["base_head_oid"], agent_main);
    let full_view_intent = api_request(
        router(state.clone()),
        "POST",
        "/v1/repos/owner/repo/push-intents",
        Some(&member_bearer()),
        Some(&serde_json::json!({ "head_oid": head, "view": "private" }).to_string()),
    )
    .await;
    assert_eq!(full_view_intent.status(), StatusCode::FORBIDDEN);

    let token = intent["token"].as_str().unwrap();
    let pushed = checkout.push(
        "refs/heads/main",
        &[format!("X-Scope-Push-Intent: {token}")],
    );
    assert!(
        pushed.status.success(),
        "{}",
        String::from_utf8_lossy(&pushed.stderr)
    );

    let name = main_push_request_names(&head).next().unwrap();
    let request = state
        .metadata
        .requests()
        .request_by_name(TEST_REPO_ID, &name)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(request.view, view("agent"));
    assert_eq!(request.title, "Main push from agent-member");
    assert_eq!(request.state(), RequestState::Open);
    assert_eq!(request.head_oid, head);
    assert!(
        state
            .metadata
            .requests()
            .request_auto_merge_intent(&request.id)
            .await
            .unwrap()
            .is_some_and(|intent| intent.is_active())
    );

    crate::use_cases::request_auto_merge::reconcile_once(&state, unix_now())
        .await
        .unwrap();
    let merged = state
        .metadata
        .requests()
        .request_by_id(&request.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(merged.state(), RequestState::Merged);
    assert_eq!(
        live_file_content(&state, "/src/main.rs").await.as_deref(),
        Some("fn main() { run() }\n")
    );
    assert!(contains_commit(
        &checkout.path,
        &head,
        &checkout.fetch_main()
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_retried_main_push_reuses_its_open_request_and_a_closed_one_moves_to_the_next_name() {
    let (state, _head, _source) = fixture("agent-main-push-retry").await;
    grant_push(&state).await;
    let checkout = agent_checkout(&state, "agent-main-push-retry-clone").await;
    let head = checkout.commit(&[("src/main.rs", "fn main() { retry() }\n")], "agent main");
    let names = main_push_request_names(&head).take(2).collect::<Vec<_>>();

    push_main(&state, &checkout, &head).await;
    push_main(&state, &checkout, &head).await;
    let requests = main_push_requests(&state).await;
    assert_eq!(
        requests
            .iter()
            .map(|request| request.name.as_str())
            .collect::<Vec<_>>(),
        [names[0].as_str()]
    );

    expect_json(
        api_request(
            router(state.clone()),
            "DELETE",
            &format!("/v1/repos/{TEST_REPO_ID}/requests/{}", requests[0].id),
            Some(&member_bearer()),
            None,
        )
        .await,
        StatusCode::OK,
    )
    .await;
    push_main(&state, &checkout, &head).await;
    let requests = main_push_requests(&state).await;
    assert_eq!(
        requests
            .iter()
            .map(|request| (request.name.as_str(), request.state()))
            .collect::<Vec<_>>(),
        [
            (names[0].as_str(), RequestState::Closed),
            (names[1].as_str(), RequestState::Open),
        ]
    );
    assert_eq!(requests[1].head_oid, head);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_request_transaction_refuses_a_duplicate_or_stale_main_push() {
    let (state, _head, _source) = fixture("agent-main-push-race").await;
    grant_push(&state).await;
    let checkout = agent_checkout(&state, "agent-main-push-race-clone").await;
    let agent_main_before = checkout.fetch_main();
    let head = checkout.commit(&[("src/main.rs", "fn main() { race() }\n")], "agent main");
    let names = main_push_request_names(&head).take(2).collect::<Vec<_>>();
    push_main(&state, &checkout, &head).await;
    let landed = main_push_requests(&state).await.remove(0);
    let incarnation_id = stored_repo_incarnation(&state).await;

    let duplicate = state
        .metadata
        .requests()
        .start_main_push_request(StartMainPushRequestInput {
            repository_incarnation_id: incarnation_id.clone(),
            ..concurrent_push(&landed, "req_duplicate", &names[0], &agent_main_before)
        })
        .await
        .unwrap();
    assert_eq!(
        duplicate,
        MainPushRequestOutcome::AlreadyOpen(Box::new(landed.clone()))
    );
    assert_eq!(main_push_requests(&state).await.len(), 1);

    crate::use_cases::request_auto_merge::reconcile_once(&state, unix_now())
        .await
        .unwrap();
    assert_ne!(checkout.fetch_main(), agent_main_before);
    let stale = state
        .metadata
        .requests()
        .start_main_push_request(StartMainPushRequestInput {
            repository_incarnation_id: incarnation_id,
            ..concurrent_push(&landed, "req_stale", &names[1], &agent_main_before)
        })
        .await
        .unwrap_err();
    assert_eq!(
        stale.message,
        "the Agent view's main moved; pull it, then push again"
    );
    assert_eq!(main_push_requests(&state).await.len(), 1);
}

async fn stored_repo_incarnation(state: &AppState) -> String {
    super::custom_views::stored_repo(state)
        .await
        .incarnation()
        .incarnation_id()
        .to_string()
}

async fn prepare_agent_main_push(
    state: &AppState,
    head: &str,
) -> git_receive_use_case::ReceivePreparation {
    let mut headers = HeaderMap::new();
    headers.insert("authorization", member_bearer().parse().unwrap());
    headers.insert(
        "x-scope-push-intent",
        view_push_intent(state, head).await.parse().unwrap(),
    );
    let (authorization, push_intent) = crate::git::receive_pack_credentials(state, &headers)
        .await
        .unwrap();
    let access = git_receive_use_case::authorize(
        state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        &view("agent"),
        authorization,
        push_intent.as_deref(),
    )
    .await
    .unwrap();
    git_receive_use_case::prepare(state, TEST_REPO_OWNER, TEST_REPO_NAME, access, false)
        .await
        .unwrap()
}

async fn complete_agent_main_push(
    state: &AppState,
    preparation: git_receive_use_case::ReceivePreparation,
) -> Result<git_receive_use_case::ReceiveCompletion, crate::error::ApiError> {
    let staging_repo = preparation.staging_repo.clone();
    let completion = git_receive_use_case::complete(
        state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        &staging_repo,
        preparation,
        std::time::Duration::ZERO,
    )
    .await;
    let _ = fs::remove_dir_all(staging_repo);
    completion
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_main_push_that_leaves_the_view_main_where_it_is_creates_no_request() {
    let (state, _head, _source) = fixture("agent-main-push-unchanged").await;
    grant_push(&state).await;
    let checkout = agent_checkout(&state, "agent-main-push-unchanged-clone").await;
    let agent_main = checkout.fetch_main();

    let unchanged = prepare_agent_main_push(&state, &agent_main).await;
    assert_eq!(
        complete_agent_main_push(&state, unchanged).await.unwrap(),
        git_receive_use_case::ReceiveCompletion::NoChange
    );
    assert!(main_push_requests(&state).await.is_empty());

    let head = checkout.commit(&[("src/main.rs", "fn main() { same() }\n")], "agent main");
    let overtaken = prepare_agent_main_push(&state, &head).await;
    push_main(&state, &checkout, &head).await;
    crate::use_cases::request_auto_merge::reconcile_once(&state, unix_now())
        .await
        .unwrap();
    assert_eq!(checkout.fetch_main(), head);
    for args in [
        vec!["fetch", checkout.path.to_str().unwrap(), head.as_str()],
        vec!["update-ref", "refs/heads/main", head.as_str()],
    ] {
        run_git(
            Some(&overtaken.staging_repo),
            &args,
            "receive the landed head again",
        )
        .unwrap();
    }
    let refused = complete_agent_main_push(&state, overtaken)
        .await
        .unwrap_err();
    assert_eq!(
        refused.kind,
        crate::error::ErrorKind::Conflict,
        "{refused:?}"
    );
    assert_eq!(main_push_requests(&state).await.len(), 1);
}
