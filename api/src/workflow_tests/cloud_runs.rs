use super::*;
use scope_api_contract::{
    AttemptCacheKey, AttemptStatusResponse, ClaimRuntimeResponse, CompleteAttemptStepRequest,
    RestoreAttemptCacheRequest, StepConclusionRequest,
};

#[tokio::test]
async fn cloud_runtime_claim_is_one_use_and_completes_the_job() {
    let mut state = test_state_with_repo();
    let (analytics, recording) = scope_product_analytics::ProductAnalytics::recording();
    state.product_analytics = analytics;
    state.dispatch_broker_token = Some(Arc::from("broker-secret"));
    cache_test_jwks(&state);
    allow_native_runs(&state).await;
    let app = router(state.clone());
    let source = temp_git_repo("cloud-runtime-protocol");
    fs::create_dir_all(source.join(".scope/runs")).unwrap();
    fs::write(
        source.join(".scope/runs/test.yml"),
        workflow_named("Cloud protocol"),
    )
    .unwrap();
    run_git(Some(&source), &["add", "."], "stage cloud run source").unwrap();
    commit_all(&source, "cloud run source");
    let (git_oid, bundle) = super::run_resources::head_bundle(&source);
    let resolved = api_request(
        app.clone(),
        "POST",
        &format!(
            "{}?workflow=test&git_oid={git_oid}&request_id=11111111111111111111111111111111",
            scope_api_contract::routes::repo_run_resolve(TEST_REPO_OWNER, TEST_REPO_NAME)
        ),
        Some(&bearer_header()),
        None,
    )
    .await;
    assert_eq!(resolved.status(), StatusCode::OK);
    assert_eq!(response_json(resolved).await["status"], "upload-required");
    assert!(
        state
            .metadata
            .runs()
            .run("run_11111111111111111111111111111111")
            .await
            .unwrap()
            .is_none()
    );
    let upload = super::run_resources::manual_run_upload(
        &git_oid,
        "11111111111111111111111111111111",
        bundle.clone(),
        bearer_header(),
    );
    let created = app.clone().oneshot(upload).await.unwrap();
    assert_eq!(created.status(), StatusCode::OK);
    let created = response_json(created).await;
    let run_id = created["id"].as_str().unwrap().to_string();

    let attempt_id = "attempt_cloud_protocol";
    let bootstrap_token = format!("scope_bootstrap_{}", "test-token");
    let scope_postgres::db::DispatchAdmission::Admitted(offer) = state
        .metadata
        .runs()
        .admit_next_job(
            1,
            attempt_id,
            &machine_token_hash(&bootstrap_token),
            "test-runtime",
            unix_now(),
            unix_now() + 900,
        )
        .await
        .unwrap()
    else {
        panic!("expected the queued job to be admitted");
    };

    let authorize_request = || {
        let body = serde_json::json!({
            "action": "start",
            "attempt_id": attempt_id,
            "bootstrap_token": bootstrap_token,
        });
        Request::builder()
            .method("POST")
            .uri(scope_api_contract::routes::DISPATCH_AUTHORIZE)
            .header(AUTHORIZATION, "Bearer broker-secret")
            .header(CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap()
    };
    let authorized = app.clone().oneshot(authorize_request()).await.unwrap();
    assert_eq!(authorized.status(), StatusCode::OK);
    let authorized = response_json(authorized).await;
    assert_eq!(authorized["action"], "start");
    assert_eq!(authorized["attempt_id"], attempt_id);
    assert_eq!(
        authorized["image"],
        offer.job.pinned_container_image.as_str()
    );
    assert!(authorized["deadline_unix"].as_u64().unwrap() > unix_now());

    let claim_request = || {
        Request::builder()
            .method("POST")
            .uri(scope_api_contract::routes::attempt_claim(attempt_id))
            .header(AUTHORIZATION, format!("Bearer {bootstrap_token}"))
            .body(Body::empty())
            .unwrap()
    };
    let claimed = app.clone().oneshot(claim_request()).await.unwrap();
    assert_eq!(claimed.status(), StatusCode::OK);
    let claim: ClaimRuntimeResponse = serde_json::from_value(response_json(claimed).await).unwrap();
    assert!(claim.attempt_token.starts_with("scope_attempt_"));
    let authorization_replayed = app.clone().oneshot(authorize_request()).await.unwrap();
    assert_eq!(authorization_replayed.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        claim.job.pinned_container_image,
        "alpine@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    );

    let replayed = app.clone().oneshot(claim_request()).await.unwrap();
    assert_eq!(replayed.status(), StatusCode::UNAUTHORIZED);

    let attempt_auth = format!("Bearer {}", claim.attempt_token);
    let undeclared_cache_restore = serde_json::to_string(&RestoreAttemptCacheRequest {
        cache: AttemptCacheKey {
            cache_name: "cargo".to_string(),
            compatibility_inputs_digest: "a".repeat(64),
            exact_inputs_digest: "b".repeat(64),
        },
    })
    .unwrap();
    let cache_restore_path = scope_api_contract::routes::attempt_cache_restore(attempt_id);
    let cache_restore = || {
        api_request(
            app.clone(),
            "POST",
            &cache_restore_path,
            Some(&attempt_auth),
            Some(&undeclared_cache_restore),
        )
    };
    assert_eq!(cache_restore().await.status(), StatusCode::BAD_REQUEST);
    let source_response = get(
        app.clone(),
        &scope_api_contract::routes::attempt_source(attempt_id),
        Some(&attempt_auth),
    )
    .await;
    assert_eq!(source_response.status(), StatusCode::OK);
    assert_eq!(
        source_response.headers()["x-scope-source-identity"],
        offer.run.source.source_identity()
    );
    assert_eq!(
        source_response.headers()["x-scope-source-sha256"],
        offer.run.source.source_identity()
    );
    assert_eq!(
        to_bytes(source_response.into_body(), 4 * 1024 * 1024)
            .await
            .unwrap()
            .as_ref(),
        bundle
    );

    let heartbeat = api_request(
        app.clone(),
        "POST",
        &scope_api_contract::routes::attempt_heartbeat(attempt_id),
        Some(&attempt_auth),
        Some("{}"),
    )
    .await;
    assert_eq!(heartbeat.status(), StatusCode::OK);
    let heartbeat: AttemptStatusResponse =
        serde_json::from_value(response_json(heartbeat).await).unwrap();
    assert!(heartbeat.lease_expires_at_unix > unix_now());

    let started = api_request(
        app.clone(),
        "POST",
        &scope_api_contract::routes::attempt_step_start(attempt_id, 0),
        Some(&attempt_auth),
        Some("{}"),
    )
    .await;
    assert_eq!(started.status(), StatusCode::OK);

    let completed_step = api_request(
        app.clone(),
        "POST",
        &scope_api_contract::routes::attempt_step_complete(attempt_id, 0),
        Some(&attempt_auth),
        Some(
            &serde_json::to_string(&CompleteAttemptStepRequest {
                conclusion: StepConclusionRequest::Succeeded,
                logs_truncated: false,
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(completed_step.status(), StatusCode::OK);
    assert_eq!(response_json(completed_step).await["state"], "running");

    let completion_body = serde_json::to_string(&scope_api_contract::CompleteAttemptRequest {
        conclusion: scope_api_contract::AttemptConclusionRequest::Succeeded,
        logs_truncated: false,
    })
    .unwrap();
    let completed_attempt = api_request(
        app.clone(),
        "POST",
        &scope_api_contract::routes::attempt_complete(attempt_id),
        Some(&attempt_auth),
        Some(&completion_body),
    )
    .await;
    assert_eq!(completed_attempt.status(), StatusCode::OK);
    assert_eq!(response_json(completed_attempt).await["state"], "succeeded");
    let replayed_completion = api_request(
        app.clone(),
        "POST",
        &scope_api_contract::routes::attempt_complete(attempt_id),
        Some(&attempt_auth),
        Some(&completion_body),
    )
    .await;
    assert_eq!(replayed_completion.status(), StatusCode::OK);
    assert_eq!(recording.event_names(), ["workflow:attempt_complete"]);
    assert_eq!(
        recording.property(0, "repository_id"),
        Some(serde_json::Value::String("repoi_workflow_test".into()))
    );
    assert_eq!(
        recording.property(0, "attempt_id"),
        Some(serde_json::Value::String(attempt_id.into()))
    );
    assert_eq!(recording.property(0, "attempt_number"), Some(1.into()));
    assert_eq!(
        recording.property(0, "actor_type"),
        Some(serde_json::Value::String("user".into()))
    );
    assert_eq!(
        recording.property(0, "result"),
        Some(serde_json::Value::String("succeeded".into()))
    );
    assert_eq!(
        recording.property(0, "run_result"),
        Some(serde_json::Value::String("succeeded".into()))
    );
    assert_eq!(cache_restore().await.status(), StatusCode::UNAUTHORIZED);

    let retried = api_request(
        app.clone(),
        "POST",
        &scope_api_contract::routes::repo_run_retry(TEST_REPO_OWNER, TEST_REPO_NAME, &run_id),
        Some(&bearer_header()),
        None,
    )
    .await;
    assert_eq!(retried.status(), StatusCode::OK);
    assert_eq!(response_json(retried).await["state"], "queued");

    let canceled = api_request(
        app,
        "POST",
        &scope_api_contract::routes::repo_run_cancel(TEST_REPO_OWNER, TEST_REPO_NAME, &run_id),
        Some(&bearer_header()),
        None,
    )
    .await;
    assert_eq!(canceled.status(), StatusCode::OK);
    assert_eq!(response_json(canceled).await["state"], "canceled");
}
