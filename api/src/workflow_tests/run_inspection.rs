use super::*;
use scope_domain::runs::{
    log::RunLogChunk,
    run::Run,
    source::{RunSource, RunTrigger},
    step::StepConclusion,
};

struct InspectableRun {
    state: AppState,
    run_id: String,
    attempt_id: String,
    first_log_position: u64,
    second_log_position: u64,
}

async fn active_run(log_chunks: usize) -> InspectableRun {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    allow_native_runs(&state).await;
    let revision = scope_run_config::parse_workflow(
        "/.scope/runs/inspection.yml",
        workflow_named("Inspection").as_bytes(),
    )
    .unwrap()
    .into_revision(TEST_REPO_ID.to_string())
    .unwrap();
    let run_id = "run_inspection".to_string();
    let mut source =
        scope_storage::content_object_for_bytes(ContentObjectKind::GitBundle, b"inspection bundle");
    source.git_oid = "b".repeat(40);
    let run = Run::new(
        run_id.clone(),
        format!("manual:{run_id}"),
        revision.workflow().clone(),
        revision.digest(),
        RunTrigger::Manual,
        Some(test_owner_id()),
        RunSource::ephemeral_git_bundle(source).unwrap(),
        1,
    )
    .unwrap();
    let runs = state.metadata.runs();
    runs.enqueue_run(run, revision).await.unwrap();

    let attempt_id = format!("attempt_{run_id}");
    let bootstrap_token_hash = "c".repeat(64);
    let attempt_token_hash = "d".repeat(64);
    runs.dispatch_job(
        &run_id,
        "checks",
        &attempt_id,
        &bootstrap_token_hash,
        "test-runtime",
        2,
        100,
    )
    .await
    .unwrap();
    runs.claim_runtime(
        &attempt_id,
        &bootstrap_token_hash,
        &attempt_token_hash,
        3,
        100,
    )
    .await
    .unwrap();
    runs.start_attempt_step(&attempt_id, &attempt_token_hash, 0, 4)
        .await
        .unwrap();
    let mut positions = Vec::new();
    for sequence in 1..=log_chunks as u64 {
        let (text, at) = match sequence {
            1 => ("first\n".to_string(), 5),
            2 => ("second\n".to_string(), 6),
            _ => ("x".repeat(64 * 1024), 6),
        };
        let chunk = RunLogChunk::new(&attempt_id, 0, sequence, text, at).unwrap();
        let appended = runs
            .append_attempt_log(chunk, &attempt_token_hash, at)
            .await
            .unwrap();
        positions.push(appended.log.position);
    }

    InspectableRun {
        state,
        run_id,
        attempt_id,
        first_log_position: positions[0],
        second_log_position: positions[1],
    }
}

#[tokio::test]
async fn run_detail_reconstructs_jobs_attempts_and_steps() {
    let fixture = inspectable_run(false, 2).await;
    let uri = scope_api_contract::routes::repo_run_detail(
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        &fixture.run_id,
    );
    let response = get_as(&fixture.state, &uri, &bearer_header()).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["run"]["id"], fixture.run_id);
    assert_eq!(body["jobs"][0]["job"]["key"], "checks");
    assert_eq!(body["jobs"][0]["job"]["started_at_unix"], 4);
    assert_eq!(body["jobs"][0]["attempts"][0]["id"], fixture.attempt_id);
    assert_eq!(body["jobs"][0]["attempts"][0]["steps"][0]["name"], "Test");
    assert_eq!(
        body["jobs"][0]["attempts"][0]["steps"][0]["command"],
        "printf 'hello from runner\\n'"
    );
}

#[tokio::test]
async fn step_log_inspection_applies_the_cursor_and_preserves_truncation() {
    let fixture = inspectable_run(true, 2).await;
    let uri = format!(
        "{}?after={}",
        step_logs_path(&fixture),
        fixture.first_log_position
    );
    let response = get_as(&fixture.state, &uri, &bearer_header()).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["logs"].as_array().unwrap().len(), 1);
    assert_eq!(body["logs"][0]["text"], "second\n");
    assert_eq!(body["next_after"], fixture.second_log_position);
    assert_eq!(body["logs_truncated"], true);
}

#[tokio::test]
async fn long_step_logs_open_at_the_tail_and_page_back_without_gaps() {
    let fixture = inspectable_run(false, 18).await;
    let path = step_logs_path(&fixture);
    let mut query = String::new();
    let mut sequences = Vec::new();
    loop {
        let response = get_as(&fixture.state, &format!("{path}{query}"), &bearer_header()).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        let logs = body["logs"].as_array().unwrap();
        assert!(
            logs.iter()
                .map(|log| log["text"].as_str().unwrap().len())
                .sum::<usize>()
                <= 512 * 1024
        );
        if query.is_empty() {
            assert_eq!(logs.first().unwrap()["sequence"], 11);
            assert_eq!(logs.last().unwrap()["sequence"], 18);
            assert_eq!(body["has_more"], false);
        }
        let page_sequences = logs
            .iter()
            .map(|log| log["sequence"].as_u64().unwrap())
            .collect::<Vec<_>>();
        assert!(page_sequences.windows(2).all(|pair| pair[0] < pair[1]));
        sequences.extend(page_sequences);
        if body["has_earlier"] == false {
            break;
        }
        query = format!(
            "?before={}",
            logs.first().unwrap()["position"].as_u64().unwrap()
        );
    }
    sequences.sort_unstable();
    assert_eq!(sequences, (1..=18).collect::<Vec<_>>());
}

#[tokio::test]
async fn step_log_inspection_rejects_conflicting_cursors() {
    let fixture = inspectable_run(false, 2).await;
    let uri = format!("{}?before=9&after=1", step_logs_path(&fixture));
    let response = get_as(&fixture.state, &uri, &bearer_header()).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

async fn get_as(state: &AppState, uri: &str, bearer: &str) -> axum::response::Response {
    get(router(state.clone()), uri, Some(bearer)).await
}

fn step_logs_path(fixture: &InspectableRun) -> String {
    scope_api_contract::routes::repo_run_step_logs(
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        &fixture.run_id,
        &fixture.attempt_id,
        0,
    )
}

async fn inspectable_run(logs_truncated: bool, log_chunks: usize) -> InspectableRun {
    let fixture = active_run(log_chunks).await;
    let attempt_token_hash = "d".repeat(64);
    fixture
        .state
        .metadata
        .runs()
        .complete_attempt_step(
            &fixture.attempt_id,
            &attempt_token_hash,
            0,
            StepConclusion::Succeeded,
            logs_truncated,
            7,
        )
        .await
        .unwrap();
    fixture
}

#[tokio::test]
async fn append_attempt_log_rejects_nul_text_as_bad_request() {
    let fixture = active_run(2).await;
    let response = api_request(
        router(fixture.state),
        "POST",
        &scope_api_contract::routes::attempt_logs(&fixture.attempt_id),
        Some("Bearer scope_attempt_inspection"),
        Some(r#"{"step_index":0,"sequence":3,"text":"before\u0000after"}"#),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn run_inspection_enforces_repository_access() {
    let fixture = inspectable_run(false, 2).await;
    let run_path =
        scope_api_contract::routes::repo_run(TEST_REPO_OWNER, TEST_REPO_NAME, &fixture.run_id);
    let response = get_as(&fixture.state, &run_path, &bearer_header()).await;
    assert_eq!(response.status(), StatusCode::OK);

    let member_subject = "user_run_member";
    let member_id = scope_postgres::db::scope_user_id_for_auth_identity("clerk", member_subject);
    fixture
        .state
        .metadata
        .auth()
        .insert_user_for_tests(test_user(
            member_id.clone(),
            "run-member",
            "run-member@example.com",
        ))
        .await
        .unwrap();
    fixture
        .state
        .metadata
        .repositories()
        .mutate_repository_for_tests(TEST_REPO_ID, |repo| {
            repo.collaboration.members.push(test_repository_member(
                TEST_REPO_ID,
                member_id,
                RepositoryMemberPermissions::default(),
            ));
        })
        .await
        .unwrap();

    let member = bearer_header_for(member_subject, "run-member@example.com");
    let response = get_as(&fixture.state, &run_path, &member).await;
    assert_eq!(response.status(), StatusCode::OK);

    let public_subject = "user_run_public";
    let public_id = scope_postgres::db::scope_user_id_for_auth_identity("clerk", public_subject);
    fixture
        .state
        .metadata
        .auth()
        .insert_user_for_tests(test_user(public_id, "run-public", "run-public@example.com"))
        .await
        .unwrap();

    let public = bearer_header_for(public_subject, "run-public@example.com");
    let response = get_as(&fixture.state, &run_path, &public).await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let mut other = test_repo(&test_owner_id());
    other.record.id = "owner/other".to_string();
    other.record.incarnation_id = "repoi_owner_other".to_string();
    other.record.name = "other".to_string();
    other.graph.repo_id = other.record.id.clone();
    fixture
        .state
        .metadata
        .repositories()
        .replace_repository_for_tests(other)
        .await
        .unwrap();

    let other_path =
        scope_api_contract::routes::repo_run(TEST_REPO_OWNER, "other", &fixture.run_id);
    let response = get_as(&fixture.state, &other_path, &bearer_header()).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
