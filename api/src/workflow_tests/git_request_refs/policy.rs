use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn advertisement_and_exact_fetch_follow_viewer_and_publication_policy() {
    let state = test_state_with_request().await;
    let (_author_checkout, permissioned_remote, _server, request_head) =
        request_checkout(&state, "request-advertisement-source").await;
    insert_public_contributor(&state).await;
    state
        .metadata
        .requests()
        .add_request_invitee(AddRequestInviteeCommand {
            request_id: REQUEST_ID.to_string(),
            actor_user_id: public_user_id(),
            target_handle: "contributor".to_string(),
            now_unix: 3,
        })
        .await
        .unwrap();
    insert_member_user(&state).await;
    state
        .metadata
        .auth()
        .insert_user_for_tests(test_user(unrelated_user_id(), "unrelated", UNRELATED_EMAIL))
        .await
        .unwrap();
    let public_remote = permissioned_remote;
    let private_remote = public_remote.replace("/git/public/", "/git/private/");
    let author = bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL);
    let invitee = bearer_header_for(CONTRIBUTOR_SUBJECT, CONTRIBUTOR_EMAIL);
    let maintainer = bearer_header_for(MEMBER_SUBJECT, MEMBER_EMAIL);
    let unrelated = bearer_header_for(UNRELATED_SUBJECT, UNRELATED_EMAIL);

    assert!(advertises_request_ref(&public_remote, Some(&author)));
    assert!(advertises_request_ref(&public_remote, Some(&invitee)));
    assert!(!advertises_request_ref(&private_remote, Some(&maintainer)));
    assert!(!advertises_request_ref(&public_remote, Some(&unrelated)));
    assert!(!advertises_request_ref(&private_remote, Some(&author)));
    assert!(!advertises_request_ref(&public_remote, None));
    assert!(!fetch_exact_request_tip(
        &private_remote,
        Some(&maintainer),
        &request_head,
        "maintainer-draft-exact-fetch",
    ));
    assert!(!fetch_exact_request_tip(
        &public_remote,
        Some(&unrelated),
        &request_head,
        "unrelated-draft-exact-fetch",
    ));
    assert!(!fetch_exact_request_tip(
        &public_remote,
        None,
        &request_head,
        "public-draft-exact-fetch",
    ));

    state
        .metadata
        .requests()
        .submit_request(SubmitRequestCommand {
            request_id: REQUEST_ID.to_string(),
            actor_user_id: public_user_id(),
            event_id: "event_advertisement_submitted".to_string(),
            now_unix: 4,
        })
        .await
        .unwrap();
    for (remote, bearer, listed) in [
        (public_remote.as_str(), None, false),
        (public_remote.as_str(), Some(author.as_str()), true),
        (public_remote.as_str(), Some(invitee.as_str()), true),
        (private_remote.as_str(), Some(maintainer.as_str()), false),
        (public_remote.as_str(), Some(unrelated.as_str()), false),
    ] {
        assert_eq!(advertises_request_ref(remote, bearer), listed, "{bearer:?}");
        assert!(
            fetch_exact_request_tip(remote, bearer, REQUEST_NAME, "submitted-named-fetch"),
            "{bearer:?}"
        );
    }
    assert!(!lists_request_ref(&public_remote, Some(&unrelated), "0"));
}

fn unrelated_user_id() -> String {
    scope_postgres::db::scope_user_id_for_auth_identity("clerk", UNRELATED_SUBJECT)
}

fn advertises_request_ref(remote: &str, bearer: Option<&str>) -> bool {
    lists_request_ref(remote, bearer, "2")
}

fn lists_request_ref(remote: &str, bearer: Option<&str>, protocol: &str) -> bool {
    let protocol = format!("protocol.version={protocol}");
    let header = bearer.map(|bearer| format!("http.{remote}.extraHeader=Authorization: {bearer}"));
    let mut command = vec!["-c", protocol.as_str()];
    if let Some(header) = header.as_deref() {
        command.extend(["-c", header]);
    }
    command.extend(["ls-remote", remote]);
    let output = run_git_output(None, &command, "reading request ref advertisement").unwrap();
    String::from_utf8(output.stdout)
        .unwrap()
        .contains(REQUEST_REF)
}

fn fetch_exact_request_tip(remote: &str, bearer: Option<&str>, refspec: &str, label: &str) -> bool {
    let checkout = TempGitRepo(unique_test_path(label));
    run_git(
        None,
        &["init", checkout.to_str().unwrap()],
        "init exact fetch repo",
    )
    .unwrap();
    let header = bearer.map(|bearer| format!("http.{remote}.extraHeader=Authorization: {bearer}"));
    let output = match header.as_deref() {
        Some(header) => run_git_output(
            Some(&checkout),
            &["-c", header, "fetch", remote, refspec],
            "fetch exact request tip",
        ),
        None => run_git_output(
            Some(&checkout),
            &["fetch", remote, refspec],
            "fetch public exact request tip",
        ),
    }
    .unwrap();
    output.status.success()
}
