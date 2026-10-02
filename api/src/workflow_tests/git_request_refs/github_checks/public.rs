//! GitHub checks for public contributions test the contribution merged onto
//! private main, which holds files the public view lacks.

use super::{public_repositories::make_github_repository_public, *};
use crate::use_cases::public_check_commits::public_tested_commit;
use scope_domain::{
    repo_config::RepoConfigVisibilityRule,
    requests::{GitHubTestedCommit, PRIVATE_CODE_CONFLICT_MESSAGE},
};

const PRIVATE_FILE: &str = "secret.txt";
const PRIVATE_CONTENT: &str = "private code\n";

/// A repository whose `secret.txt` is private, connected to GitHub, with the
/// public user's request started on public main.
async fn private_file_repository(label: &str) -> (AppState, Arc<FakeGitHub>, TempGitRepo) {
    let mut state = test_state_with_repo();
    cache_test_jwks(&state);
    let source = temp_git_repo(label);
    fs::write(source.join("README.md"), "hello\n").unwrap();
    fs::write(source.join(PRIVATE_FILE), PRIVATE_CONTENT).unwrap();
    run_git(Some(&source), &["add", "."], "add files").unwrap();
    commit_all(&source, "initial");
    let bare = clone_test_repo(&source, &format!("{label}-bare"), true);
    let mut config = repo_config(Visibility::Public);
    config.visibility.rules.push(RepoConfigVisibilityRule {
        path: format!("/{PRIVATE_FILE}"),
        visibility: ConfigVisibility::Private,
    });
    apply_first_push_from_staging_repo(&state, &bare, config).await;
    state
        .metadata
        .auth()
        .insert_user_for_tests(test_user(public_user_id(), "public", PUBLIC_EMAIL))
        .await
        .unwrap();
    start_public_request(&state).await;
    insert_member_user(&state).await;
    let fake = connect_github(&mut state, &[REQUIRED_CHECK]).await;
    (state, fake, source)
}

async fn submit_public_request(state: &AppState) {
    let submitted = api_request(
        router(state.clone()),
        "POST",
        &format!("/v1/repos/{TEST_REPO_ID}/requests/{REQUEST_ID}/submit"),
        Some(&bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL)),
        Some("{}"),
    )
    .await;
    assert_eq!(submitted.status(), StatusCode::OK);
}

async fn approve(state: &AppState, head: &str) -> Response {
    api_request(
        router(state.clone()),
        "POST",
        &repo_request_checks_approve(TEST_REPO_OWNER, TEST_REPO_NAME, REQUEST_ID),
        Some(&bearer_header_for(MEMBER_SUBJECT, MEMBER_EMAIL)),
        Some(&serde_json::json!({ "expected_head_oid": head }).to_string()),
    )
    .await
}

fn file_at(repo: &FsPath, oid: &str, path: &str) -> String {
    git_stdout_text(
        repo,
        &["show", &format!("{oid}:{path}")],
        "read pushed file",
    )
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_public_contribution_is_tested_merged_onto_private_main() {
    let (state, fake, _owner_source) = private_file_repository("github-public-check").await;
    let (source, remote, _server) = request_push_checkout(
        &state,
        "github-public-check-push",
        PUBLIC_SUBJECT,
        PUBLIC_EMAIL,
    )
    .await;
    assert!(!source.join(PRIVATE_FILE).exists());
    push_change(
        &source,
        &remote,
        REQUEST_REF,
        "request.txt",
        "contribution\n",
        "contribute",
    )
    .unwrap();
    let head = git_head_oid(&source);
    submit_public_request(&state).await;
    let branch = format!("scope/requests/{REQUEST_ID}");

    // Nothing leaves Scope before a maintainer approves.
    assert_eq!(push_pass(&state, unix_now()).await, 0);
    let evaluation = state
        .metadata
        .requests()
        .request_check_evaluation(REQUEST_ID, &head)
        .await
        .unwrap()
        .unwrap();
    let tested = evaluation.tested_oid.clone();
    assert_ne!(tested, head);
    let base = evaluation.check_commit_base.clone().unwrap();

    expect_json(approve(&state, &head).await, StatusCode::OK).await;
    assert_eq!(push_pass(&state, unix_now()).await, 1);
    assert_eq!(fake.branch_head(&branch), Some(tested.clone()));

    // The pushed commit holds the private file and the contribution, merged
    // onto private main with the head as its second parent.
    let github = fake.repository_path();
    assert_eq!(file_at(&github, &tested, PRIVATE_FILE), PRIVATE_CONTENT);
    assert_eq!(file_at(&github, &tested, "request.txt"), "contribution\n");
    let parents = git_stdout_text(
        &github,
        &["show", "-s", "--format=%P", &tested],
        "read check commit parents",
    )
    .unwrap();
    assert_eq!(
        parents.split_whitespace().collect::<Vec<_>>(),
        [base.private_main_oid.as_str(), head.as_str()]
    );

    // Building it again from the same head and private main gives the same commit.
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    let request = stored_request(&state, REQUEST_ID).await;
    let revision = state
        .metadata
        .requests()
        .request_revision_with_head(REQUEST_ID, &head)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        public_tested_commit(&state, &repo, &request, &revision)
            .await
            .unwrap(),
        GitHubTestedCommit::CheckCommit {
            oid: tested.clone(),
            base: base.clone(),
        }
    );

    // GitHub's results for the check commit decide the merge.
    fake.report_check_runs(
        &tested,
        vec![check_run(1, REQUIRED_CHECK, &tested, Some("success"))],
    );
    deliver_check_run(&state, GITHUB_REPOSITORY_ID, &tested).await;
    let member = bearer_header_for(MEMBER_SUBJECT, MEMBER_EMAIL);
    let maintainer_view = checks(&state, REQUEST_ID, &member).await;
    assert_eq!(maintainer_view["mergeability"]["status"], "Ready");
    assert_eq!(maintainer_view["checks"][0]["conclusion"], "success");

    // Only the head is named to the contributor; the private commit is not.
    let public = bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL);
    let contributor_view = checks(&state, REQUEST_ID, &public).await;
    assert_eq!(contributor_view["head_oid"], head);
    assert_eq!(contributor_view["checks"][0]["conclusion"], "success");
    let request_view = expect_json(
        api_request(
            router(state.clone()),
            "GET",
            &scope_api_contract::routes::repo_request(TEST_REPO_OWNER, TEST_REPO_NAME, REQUEST_ID),
            Some(&public),
            None,
        )
        .await,
        StatusCode::OK,
    )
    .await;
    for view in [&contributor_view, &request_view] {
        for private_oid in [&tested, &base.private_main_oid] {
            assert!(!view.to_string().contains(private_oid.as_str()), "{view}");
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_contribution_that_conflicts_with_private_code_reports_it_and_pushes_nothing() {
    let (state, fake, owner_source) = private_file_repository("github-public-conflict").await;
    let (source, remote, _server) = request_push_checkout(
        &state,
        "github-public-conflict-push",
        PUBLIC_SUBJECT,
        PUBLIC_EMAIL,
    )
    .await;

    // Main changes the line the contribution changes after the contributor
    // cloned it.
    fs::write(owner_source.join("README.md"), "maintainer line\n").unwrap();
    run_git(
        Some(&owner_source),
        &["add", "README.md"],
        "stage main change",
    )
    .unwrap();
    commit_all(&owner_source, "change main");
    configure_bearer_header(&owner_source, &remote, &bearer_header());
    configure_push_intent_header(&state, &owner_source, &remote, &test_owner_id()).await;
    run_git(
        Some(&owner_source),
        &["push", &remote, "HEAD:refs/heads/main"],
        "push main change",
    )
    .unwrap();
    drain_outbox(&state, "github-public-conflict-main").await;

    push_change(
        &source,
        &remote,
        REQUEST_REF,
        "README.md",
        "contributor line\n",
        "contribute",
    )
    .unwrap();
    submit_public_request(&state).await;

    let member = bearer_header_for(MEMBER_SUBJECT, MEMBER_EMAIL);
    let view = checks(&state, REQUEST_ID, &member).await;
    assert_eq!(view["state"], "configuration-error");
    assert_eq!(view["message"], PRIVATE_CODE_CONFLICT_MESSAGE);
    assert_eq!(view["can_approve"], false);
    assert_eq!(view["github_push"], serde_json::Value::Null);
    assert_eq!(view["mergeability"]["status"], "ChecksConfigurationError");
    assert_eq!(
        approve(&state, &git_head_oid(&source)).await.status(),
        StatusCode::CONFLICT
    );
    assert_eq!(push_pass(&state, unix_now()).await, 0);
    assert_eq!(
        fake.branch_head(&format!("scope/requests/{REQUEST_ID}")),
        None
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_check_commit_is_private_code_and_waits_for_a_public_repository_to_be_confirmed() {
    let (state, fake, _owner_source) = private_file_repository("github-public-check-public").await;
    let (source, remote, _server) = request_push_checkout(
        &state,
        "github-public-check-public-push",
        PUBLIC_SUBJECT,
        PUBLIC_EMAIL,
    )
    .await;
    push_change(
        &source,
        &remote,
        REQUEST_REF,
        "request.txt",
        "contribution\n",
        "contribute",
    )
    .unwrap();
    let head = git_head_oid(&source);
    submit_public_request(&state).await;
    expect_json(approve(&state, &head).await, StatusCode::OK).await;
    make_github_repository_public(&fake);
    let branch = format!("scope/requests/{REQUEST_ID}");

    // The push asks GitHub first and holds the check commit, as it would a
    // private request's head.
    assert_eq!(push_pass(&state, unix_now()).await, 1);
    assert_eq!(fake.branch_head(&branch), None);
    let held = checks(
        &state,
        REQUEST_ID,
        &bearer_header_for(MEMBER_SUBJECT, MEMBER_EMAIL),
    )
    .await;
    assert_eq!(held["github_push"]["state"], "failed");
    assert_eq!(held["mergeability"]["status"], "ChecksConfigurationError");
    assert_eq!(
        held["message"],
        scope_domain::github_connection::PRIVATE_REQUESTS_WITHHELD_MESSAGE
    );

    // Once a maintainer confirms the repository may be public, it goes.
    let confirmed = api_request(
        router(state.clone()),
        "POST",
        "/v1/repos/owner/repo/github/public-confirmation",
        Some(&bearer_header()),
        None,
    )
    .await;
    expect_json(confirmed, StatusCode::OK).await;
    assert_eq!(push_pass(&state, unix_now()).await, 1);
    let tested = state
        .metadata
        .requests()
        .request_check_evaluation(REQUEST_ID, &head)
        .await
        .unwrap()
        .unwrap()
        .tested_oid;
    assert_eq!(fake.branch_head(&branch), Some(tested));
}
