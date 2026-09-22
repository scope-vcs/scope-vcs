use super::*;

const INVITED_EMAIL: &str = "invitee@example.com";

fn invitee_header() -> String {
    bearer_header_for("user_invitee", INVITED_EMAIL)
}

async fn json_request(
    state: &AppState,
    method: &str,
    path: &str,
    bearer: Option<&str>,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let body = body.map(|body| body.to_string());
    let response = api_request(router(state.clone()), method, path, bearer, body.as_deref()).await;
    let status = response.status();
    (status, response_json(response).await)
}

/// Creates an invite through the API and returns its id and a copied link token.
async fn create_invite(state: &AppState) -> (String, String) {
    let (status, body) = json_request(
        state,
        "POST",
        "/v1/repos/owner/repo/invites",
        Some(&bearer_header()),
        Some(serde_json::json!({
            "email": INVITED_EMAIL,
            "permissions": RepositoryMemberPermissions::default(),
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    // Creating an invite queues its email and hands back no link.
    assert_eq!(body["email"]["state"], "queued");
    assert!(body.get("invite_url").is_none());
    let invite_id = body["id"].as_str().unwrap().to_string();
    let token = copy_link(state, &invite_id).await;
    (invite_id, token)
}

async fn copy_link(state: &AppState, invite_id: &str) -> String {
    let (status, body) = json_request(
        state,
        "POST",
        &format!("/v1/repos/owner/repo/invites/{invite_id}/links"),
        Some(&bearer_header()),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    link_token(&body)
}

fn link_token(body: &serde_json::Value) -> String {
    let invite_url = body["invite_url"].as_str().unwrap();
    invite_url.rsplit('/').next().unwrap().to_string()
}

fn mailer(state: &AppState) -> std::sync::Arc<crate::invite_mailer::RecordingMailer> {
    match &state.invite_mailer {
        crate::invite_mailer::InviteMailer::Recording(mailer) => mailer.clone(),
        _ => panic!("tests record invite emails"),
    }
}

async fn invite_email_state(state: &AppState) -> serde_json::Value {
    let (_, members) = json_request(
        state,
        "GET",
        "/v1/repos/owner/repo/members",
        Some(&bearer_header()),
        None,
    )
    .await;
    members["invites"][0]["email"]["state"].clone()
}

async fn landing(state: &AppState, token: &str, bearer: Option<&str>) -> serde_json::Value {
    let (status, body) = json_request(
        state,
        "GET",
        &format!("/v1/repository-invites/{token}"),
        bearer,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    body
}

async fn accept(state: &AppState, token: &str, bearer: &str) -> (StatusCode, serde_json::Value) {
    json_request(
        state,
        "POST",
        &format!("/v1/repository-invites/{token}/accept"),
        Some(bearer),
        None,
    )
    .await
}

#[tokio::test]
async fn acceptance_grants_member_access_and_can_be_repeated_safely() {
    let mut state = test_state_with_repo();
    cache_test_jwks(&state);
    let (analytics, recording) = scope_product_analytics::ProductAnalytics::recording();
    state.product_analytics = analytics;
    let (_, token) = create_invite(&state).await;

    let signed_out = landing(&state, &token, None).await;
    assert_eq!(signed_out["status"], "open");
    assert_eq!(signed_out["viewer"], "signed_out");
    assert_eq!(signed_out["invited_email"], INVITED_EMAIL);
    let stranger = bearer_header_for("user_stranger", "stranger@example.com");
    let wrong_account = landing(&state, &token, Some(&stranger)).await;
    assert_eq!(wrong_account["viewer"], "wrong_account");
    assert_eq!(wrong_account["viewer_email"], "stranger@example.com");
    assert_eq!(
        accept(&state, &token, &stranger).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        landing(&state, &token, Some(&invitee_header())).await["viewer"],
        "ready"
    );

    let (status, body) = accept(&state, &token, &invitee_header()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["repo"]["access"]["actor"], "Member");
    let accepted_events = [
        "account:user_create",
        "account:user_create",
        "repository:invite_accept",
    ];
    assert_eq!(recording.event_names(), accepted_events);
    assert_eq!(
        recording.property(2, "repository_id"),
        Some(serde_json::Value::String("repoi_workflow_test".into()))
    );
    assert_eq!(
        recording.property(2, "actor_role"),
        Some(serde_json::Value::String("member".into()))
    );

    // A double click or a retry after a lost response succeeds again without
    // adding a second membership or a second analytics event.
    let (status, repeated) = accept(&state, &token, &invitee_header()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(repeated["member"], body["member"]);
    assert_eq!(recording.event_names(), accepted_events);

    assert_eq!(
        landing(&state, &token, Some(&invitee_header())).await["status"],
        "member"
    );
    // A used link tells anyone else only that it was used.
    assert_eq!(
        landing(&state, &token, Some(&stranger)).await,
        serde_json::json!({ "status": "used" })
    );
    assert_eq!(
        landing(&state, &token, None).await,
        serde_json::json!({ "status": "used" })
    );
}

#[tokio::test]
async fn every_issued_link_works_until_the_invite_is_revoked() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let (invite_id, first) = create_invite(&state).await;
    let links_path = format!("/v1/repos/owner/repo/invites/{invite_id}/links");

    let (status, body) =
        json_request(&state, "POST", &links_path, Some(&bearer_header()), None).await;
    assert_eq!(status, StatusCode::OK);
    let second = link_token(&body);
    assert_ne!(first, second);
    let first_landing = landing(&state, &first, None).await;
    assert_eq!(first_landing["status"], "open");
    // A new link does not extend the invite.
    assert_eq!(landing(&state, &second, None).await, first_landing);

    // Only the owner can issue links. The repository is private, so anyone
    // else is told it does not exist.
    let (status, _) =
        json_request(&state, "POST", &links_path, Some(&invitee_header()), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // A second invite for the same email is refused while one is pending.
    let (status, _) = json_request(
        &state,
        "POST",
        "/v1/repos/owner/repo/invites",
        Some(&bearer_header()),
        Some(serde_json::json!({
            "email": INVITED_EMAIL,
            "permissions": RepositoryMemberPermissions::default(),
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    let (status, revoked) = json_request(
        &state,
        "DELETE",
        &format!("/v1/repos/owner/repo/invites/{invite_id}"),
        Some(&bearer_header()),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(revoked["state"], "Revoked");

    for token in [&first, &second] {
        assert_eq!(
            landing(&state, token, Some(&invitee_header())).await,
            serde_json::json!({ "status": "revoked" })
        );
        assert_eq!(
            accept(&state, token, &invitee_header()).await.0,
            StatusCode::CONFLICT
        );
    }
    let (status, _) = json_request(&state, "POST", &links_path, Some(&bearer_header()), None).await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn an_expired_invite_reads_as_expired_everywhere_and_can_be_replaced() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let token = "expired-invite-token";
    let now = unix_now();
    let invite = RepositoryInvite {
        id: "invite_expired".into(),
        repo_id: TEST_REPO_ID.into(),
        invited_email: INVITED_EMAIL.into(),
        invited_email_normalized: INVITED_EMAIL.into(),
        permissions: RepositoryMemberPermissions::default(),
        invited_by_user_id: test_owner_id(),
        link_hashes: vec![token_hash(token)],
        created_at_unix: now - 700,
        updated_at_unix: now - 700,
        expires_at_unix: now - 100,
        accepted_by_user_id: None,
        accepted_at_unix: None,
        revoked_at_unix: None,
    };
    state
        .metadata
        .repositories()
        .mutate_repository_for_tests(TEST_REPO_ID, move |repo| repo.invitations.push(invite))
        .await
        .unwrap();

    let expired = landing(&state, token, Some(&invitee_header())).await;
    assert_eq!(expired["status"], "expired");
    assert_eq!(expired["repo_name"], "repo");
    assert!(expired.get("invited_email").is_none());
    assert_eq!(
        accept(&state, token, &invitee_header()).await.0,
        StatusCode::CONFLICT
    );
    let (_, members) = json_request(
        &state,
        "GET",
        "/v1/repos/owner/repo/members",
        Some(&bearer_header()),
        None,
    )
    .await;
    assert_eq!(members["invites"][0]["state"], "Expired");

    // The expired invite does not block a new one, and its link stays dead.
    let (_, fresh) = create_invite(&state).await;
    assert_eq!(landing(&state, &fresh, None).await["status"], "open");
    assert_eq!(landing(&state, token, None).await["status"], "expired");
}

#[tokio::test]
async fn a_removed_member_cannot_rejoin_by_replaying_the_link() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let (_, token) = create_invite(&state).await;
    let (status, body) = accept(&state, &token, &invitee_header()).await;
    assert_eq!(status, StatusCode::OK);
    let member_user_id = body["member"]["user_id"].as_str().unwrap();

    let (status, _) = json_request(
        &state,
        "DELETE",
        &format!("/v1/repos/owner/repo/members/{member_user_id}"),
        Some(&bearer_header()),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    assert_eq!(
        landing(&state, &token, Some(&invitee_header())).await,
        serde_json::json!({ "status": "access_removed" })
    );
    assert_eq!(
        accept(&state, &token, &invitee_header()).await.0,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn an_unknown_link_lands_as_invalid() {
    let state = test_state_with_repo();
    assert_eq!(
        landing(&state, "scope_invite_unknown", None).await,
        serde_json::json!({ "status": "invalid" })
    );
}

use crate::use_cases::invite_email_delivery::deliver_due_invite_emails;
use scope_domain::repo_invite_email::InviteEmailAttempt;

async fn deliver_at(state: &AppState, now: u64) -> Result<usize, crate::error::ApiError> {
    deliver_due_invite_emails(state, &move || Ok(now)).await
}

/// The link inside a recorded email's text body.
fn emailed_token(text: &str) -> String {
    let link = text
        .split_whitespace()
        .find(|word| word.contains("/invites/"))
        .unwrap();
    link.rsplit('/').next().unwrap().to_string()
}

#[tokio::test]
async fn a_new_invite_is_emailed_with_a_link_that_was_never_stored() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let (invite_id, copied) = create_invite(&state).await;
    assert_eq!(invite_email_state(&state).await, "queued");
    let mut events = state.repo_events.subscribe(TEST_REPO_ID);

    assert_eq!(deliver_at(&state, unix_now()).await.unwrap(), 1);

    let sent = mailer(&state).sent.lock().unwrap().clone();
    let [(to, reply_to, text)] = sent.as_slice() else {
        panic!("one email should have been sent, got {}", sent.len());
    };
    assert_eq!(to, INVITED_EMAIL);
    assert_eq!(reply_to, TEST_OWNER_EMAIL);
    assert!(text.contains("owner/repo"));
    // The members list hears about the link and then about the delivery.
    assert!(events.recv().await.unwrap().version < events.recv().await.unwrap().version);
    assert_eq!(invite_email_state(&state).await, "sent");

    // The emailed link is its own link, and the copied one still works.
    let emailed = emailed_token(text);
    assert_ne!(emailed, copied);
    for token in [&emailed, &copied] {
        assert_eq!(landing(&state, token, None).await["status"], "open");
    }
    // Only hashes are stored: the plain token appears in no invite row.
    let (repo, _) = state
        .metadata
        .repositories()
        .repository_collaboration("owner", "repo")
        .await
        .unwrap()
        .unwrap();
    let invite = repo.invitations.iter().find(|i| i.id == invite_id).unwrap();
    assert_eq!(invite.link_hashes.len(), 2);
    assert!(
        invite
            .link_hashes
            .iter()
            .all(|hash| !hash.contains(&emailed))
    );

    // Nothing is due any more, and a second email inside a minute is refused.
    assert_eq!(deliver_at(&state, unix_now()).await.unwrap(), 0);
    let (status, body) = json_request(
        &state,
        "POST",
        &format!("/v1/repos/owner/repo/invites/{invite_id}/emails"),
        Some(&bearer_header()),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert!(
        body["message"]
            .as_str()
            .unwrap()
            .contains("less than a minute")
    );
}

#[tokio::test]
async fn an_outage_is_retried_and_a_refusal_leaves_a_retryable_invite() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let (invite_id, _) = create_invite(&state).await;
    mailer(&state).scripted.lock().unwrap().extend([
        InviteEmailAttempt::Retryable("Resend answered 503".into()),
        InviteEmailAttempt::Refused("Resend answered 422".into()),
    ]);
    let now = unix_now();

    // The outage keeps the email queued, and it is not due again straight away.
    assert_eq!(deliver_at(&state, now).await.unwrap(), 1);
    assert_eq!(invite_email_state(&state).await, "queued");
    assert_eq!(deliver_at(&state, now).await.unwrap(), 0);

    // The refusal settles it as failed without sending anything.
    assert_eq!(deliver_at(&state, now + 3_600).await.unwrap(), 1);
    assert_eq!(invite_email_state(&state).await, "failed");
    assert!(mailer(&state).sent.lock().unwrap().is_empty());

    // The same invite can be emailed again: no second invite is needed.
    let retried = state
        .metadata
        .repositories()
        .request_repository_invite_email(
            scope_postgres::db::RequestRepositoryInviteEmailCommand {
                owner: "owner".into(),
                name: "repo".into(),
                owner_user_id: test_owner_id(),
                invite_id,
                email_id: "invite_email_retry".into(),
                now_unix: now + 3_700,
            },
            &crate::persistence_ids::generate_persistence_id,
        )
        .await
        .unwrap();
    assert_eq!(retried.value.id, "invite_email_retry");
    assert_eq!(deliver_at(&state, now + 3_700).await.unwrap(), 1);
    assert_eq!(mailer(&state).sent.lock().unwrap().len(), 1);
    assert_eq!(invite_email_state(&state).await, "sent");
}

#[tokio::test]
async fn a_queued_email_for_a_revoked_invite_is_never_sent() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let (invite_id, _) = create_invite(&state).await;
    let (status, _) = json_request(
        &state,
        "DELETE",
        &format!("/v1/repos/owner/repo/invites/{invite_id}"),
        Some(&bearer_header()),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    assert_eq!(deliver_at(&state, unix_now()).await.unwrap(), 1);

    assert!(mailer(&state).sent.lock().unwrap().is_empty());
    assert_eq!(invite_email_state(&state).await, "failed");
    assert_eq!(deliver_at(&state, unix_now()).await.unwrap(), 0);
}

#[tokio::test]
async fn one_sender_holds_an_email_until_its_claim_lapses() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    create_invite(&state).await;
    let repositories = state.metadata.repositories();
    let now = unix_now();

    // A second API process finds nothing while the first holds the claim.
    let first = repositories
        .claim_due_repository_invite_emails("claim_first", now, now + 120, 20)
        .await
        .unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!(deliver_at(&state, now).await.unwrap(), 0);

    // The first process dies. Once its claim lapses another takes over, and
    // whatever the first one reports afterwards is ignored.
    assert_eq!(deliver_at(&state, now + 121).await.unwrap(), 1);
    assert_eq!(invite_email_state(&state).await, "sent");
    let late = repositories
        .record_repository_invite_email_attempt(
            &first[0],
            "claim_first",
            InviteEmailAttempt::Refused("stale sender".into()),
            None,
            now + 122,
            &crate::persistence_ids::generate_persistence_id,
        )
        .await
        .unwrap();
    assert!(late.is_none());
    assert_eq!(invite_email_state(&state).await, "sent");
    assert_eq!(mailer(&state).sent.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn an_owner_out_of_daily_emails_still_gets_an_invite_to_copy() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let invite = |email: String| {
        let state = state.clone();
        async move {
            json_request(
                &state,
                "POST",
                "/v1/repos/owner/repo/invites",
                Some(&bearer_header()),
                Some(serde_json::json!({
                    "email": email,
                    "permissions": RepositoryMemberPermissions::default(),
                })),
            )
            .await
        }
    };
    for n in 0..scope_domain::repo_invite_email::INVITE_EMAIL_MAX_PER_OWNER_PER_DAY {
        let (status, body) = invite(format!("person{n}@example.com")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["email"]["state"], "queued");
    }

    let (status, body) = invite("one-too-many@example.com".into()).await;

    assert_eq!(status, StatusCode::OK);
    assert!(body["email"].is_null());
    let token = copy_link(&state, body["id"].as_str().unwrap()).await;
    assert_eq!(landing(&state, &token, None).await["status"], "open");
    // Asking for the email says why it cannot go out yet.
    let (status, refused) = json_request(
        &state,
        "POST",
        &format!(
            "/v1/repos/owner/repo/invites/{}/emails",
            body["id"].as_str().unwrap()
        ),
        Some(&bearer_header()),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert!(
        refused["message"]
            .as_str()
            .unwrap()
            .contains("in the last day")
    );
}
