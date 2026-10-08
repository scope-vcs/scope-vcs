use super::{public_repositories::make_github_repository_public, *};
use crate::use_cases::view_check_commits::view_tested_commit;
use scope_domain::views::ViewId;
use scope_domain::{
    repo_config::RepoConfigFileRule,
    requests::{GitHubTestedCommit, PRIVATE_CODE_CONFLICT_MESSAGE},
};
use scope_postgres::db::RebuildCheckCommitCommand;

const PRIVATE_FILE: &str = "secret.txt";
const PRIVATE_CONTENT: &str = "private code\n";

pub(super) async fn private_file_repository(
    label: &str,
) -> (AppState, Arc<FakeGitHub>, TempGitRepo) {
    let mut state = test_state_with_repo();
    cache_test_jwks(&state);
    let source = temp_git_repo(label);
    fs::write(source.join("README.md"), "hello\n").unwrap();
    fs::write(source.join(PRIVATE_FILE), PRIVATE_CONTENT).unwrap();
    run_git(Some(&source), &["add", "."], "add files").unwrap();
    commit_all(&source, "initial");
    let bare = clone_test_repo(&source, &format!("{label}-bare"), true);
    let mut config = repo_config(ViewId::public());
    config.files.rules.push(RepoConfigFileRule {
        path: format!("/{PRIVATE_FILE}"),
        view: ViewId::private(),
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

async fn push_main_change(
    state: &AppState,
    owner_source: &FsPath,
    remote: &str,
    path: &str,
    content: &str,
) {
    let remote = &remote.replace("/git/public/", "/git/private/");
    fs::write(owner_source.join(path), content).unwrap();
    run_git(Some(owner_source), &["add", path], "stage main change").unwrap();
    commit_all(owner_source, "change main");
    configure_bearer_header(owner_source, remote, &bearer_header());
    configure_push_intent_header(state, owner_source, remote, &test_owner_id()).await;
    run_git(
        Some(owner_source),
        &["push", remote, "HEAD:refs/heads/main"],
        "push main change",
    )
    .unwrap();
    drain_outbox(state, "github-public-main-change").await;
}

async fn approved_contribution(
    state: &AppState,
    label: &str,
    path: &str,
    content: &str,
) -> (TempGitRepo, String, TestServer, String) {
    let (source, remote, server) =
        request_push_checkout(state, label, PUBLIC_SUBJECT, PUBLIC_EMAIL).await;
    push_change(&source, &remote, REQUEST_REF, path, content, "contribute").unwrap();
    let head = git_head_oid(&source);
    submit_public_request(state).await;
    expect_json(approve(state, &head).await, StatusCode::OK).await;
    (source, remote, server, head)
}

async fn tested_oid(state: &AppState, head: &str) -> String {
    state
        .metadata
        .requests()
        .request_check_evaluation(REQUEST_ID, head)
        .await
        .unwrap()
        .unwrap()
        .tested_oid
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
        [base.canonical_main_oid.as_str(), head.as_str()]
    );

    let git = crate::git::repository_git::RepositoryGit::load(&state, &test_repo_incarnation())
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
    let held = state
        .metadata
        .admin()
        .lock_repository_history_for_tests()
        .await
        .unwrap();
    let rebuilt = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        view_tested_commit(&state, &git, &request, &revision),
    )
    .await
    .expect("rebuilding a check commit must not wait on history tables");
    held.rollback().await.unwrap();
    assert_eq!(
        rebuilt.unwrap(),
        GitHubTestedCommit::CheckCommit {
            oid: tested.clone(),
            base: base.clone(),
        }
    );

    fake.report_check_runs(
        &tested,
        vec![check_run(1, REQUIRED_CHECK, &tested, Some("success"))],
    );
    deliver_check_run(&state, GITHUB_REPOSITORY_ID, &tested).await;
    let member = bearer_header_for(MEMBER_SUBJECT, MEMBER_EMAIL);
    let maintainer_view = checks(&state, REQUEST_ID, &member).await;
    assert_eq!(maintainer_view["mergeability"]["status"], "Ready");
    assert_eq!(maintainer_view["checks"][0]["conclusion"], "success");

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
        for private_oid in [&tested, &base.canonical_main_oid] {
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

    push_main_change(
        &state,
        &owner_source,
        &remote,
        "README.md",
        "maintainer line\n",
    )
    .await;

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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn green_checks_on_an_older_private_main_do_not_clear_the_merge() {
    let (state, fake, owner_source) = private_file_repository("github-public-check-stale").await;
    let (_source, remote, _server, head) = approved_contribution(
        &state,
        "github-public-check-stale-push",
        "request.txt",
        "contribution\n",
    )
    .await;
    let branch = format!("scope/requests/{REQUEST_ID}");
    assert_eq!(push_pass(&state, unix_now()).await, 1);
    let old = tested_oid(&state, &head).await;
    fake.report_check_runs(
        &old,
        vec![check_run(1, REQUIRED_CHECK, &old, Some("success"))],
    );
    deliver_check_run(&state, GITHUB_REPOSITORY_ID, &old).await;
    let member = bearer_header_for(MEMBER_SUBJECT, MEMBER_EMAIL);
    assert_eq!(
        checks(&state, REQUEST_ID, &member).await["mergeability"]["status"],
        "Ready"
    );

    push_main_change(
        &state,
        &owner_source,
        &remote,
        PRIVATE_FILE,
        "new private code\n",
    )
    .await;

    let refused = merge(&state, REQUEST_ID).await;
    assert_eq!(refused.status(), StatusCode::CONFLICT);
    let renewed = tested_oid(&state, &head).await;
    assert_ne!(renewed, old);
    assert_eq!(
        checks(&state, REQUEST_ID, &member).await["mergeability"]["status"],
        "ChecksPending"
    );
    assert_eq!(push_pass(&state, unix_now()).await, 1);
    assert_eq!(fake.branch_head(&branch), Some(renewed.clone()));
    let github = fake.repository_path();
    assert_eq!(
        file_at(&github, &renewed, PRIVATE_FILE),
        "new private code\n"
    );
    assert_eq!(file_at(&github, &renewed, "request.txt"), "contribution\n");

    fake.report_check_runs(
        &renewed,
        vec![check_run(2, REQUIRED_CHECK, &renewed, Some("failure"))],
    );
    deliver_check_run(&state, GITHUB_REPOSITORY_ID, &renewed).await;
    assert_eq!(
        checks(&state, REQUEST_ID, &member).await["mergeability"]["status"],
        "ChecksFailed"
    );
    fake.report_check_runs(
        &renewed,
        vec![
            check_run(2, REQUIRED_CHECK, &renewed, Some("failure")),
            check_run(3, REQUIRED_CHECK, &renewed, Some("success")),
        ],
    );
    deliver_check_run(&state, GITHUB_REPOSITORY_ID, &renewed).await;
    assert_eq!(
        checks(&state, REQUEST_ID, &member).await["mergeability"]["status"],
        "Ready"
    );
    expect_json(merge(&state, REQUEST_ID).await, StatusCode::OK).await;
    assert_eq!(
        live_file_content(&state, "/request.txt").await.as_deref(),
        Some("contribution\n")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_late_renewal_onto_the_same_private_main_queues_no_second_push() {
    let (state, fake, owner_source) =
        private_file_repository("github-public-check-late-renewal").await;
    let (_source, remote, _server, head) = approved_contribution(
        &state,
        "github-public-check-late-renewal-push",
        "request.txt",
        "contribution\n",
    )
    .await;
    let old = tested_oid(&state, &head).await;
    push_main_change(
        &state,
        &owner_source,
        &remote,
        PRIVATE_FILE,
        "new private code\n",
    )
    .await;
    crate::use_cases::request_checks::reconcile_request_checks_once(
        &state,
        &mut Default::default(),
    )
    .await
    .unwrap();
    checks(
        &state,
        REQUEST_ID,
        &bearer_header_for(MEMBER_SUBJECT, MEMBER_EMAIL),
    )
    .await;
    let renewed = state
        .metadata
        .requests()
        .request_check_evaluation(REQUEST_ID, &head)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(renewed.tested_oid, old);

    let late = state
        .metadata
        .requests()
        .rebuild_request_check_commit(RebuildCheckCommitCommand {
            expected_canonical_main_oid: state
                .metadata
                .requests()
                .request_check_results(TEST_REPO_ID, std::slice::from_ref(&renewed))
                .await
                .unwrap()
                .canonical_main_oid,
            repository_incarnation: state
                .metadata
                .repositories()
                .repository_record(TEST_REPO_ID)
                .await
                .unwrap()
                .unwrap()
                .incarnation(),
            request_id: REQUEST_ID.into(),
            head_oid: head.clone(),
            replaced_tested_oid: renewed.tested_oid.clone(),
            tested: GitHubTestedCommit::CheckCommit {
                oid: renewed.tested_oid.clone(),
                base: renewed.check_commit_base.clone().unwrap(),
            },
            now_unix: unix_now(),
        })
        .await
        .unwrap();
    assert!(late.is_none());
    assert_eq!(push_pass(&state, unix_now()).await, 1);
    assert_eq!(
        fake.branch_head(&format!("scope/requests/{REQUEST_ID}")),
        Some(renewed.tested_oid)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_contribution_that_new_private_main_conflicts_with_reports_it() {
    let (state, fake, owner_source) =
        private_file_repository("github-public-check-new-conflict").await;
    let (_source, remote, _server, head) = approved_contribution(
        &state,
        "github-public-check-new-conflict-push",
        "README.md",
        "contributor line\n",
    )
    .await;
    assert_eq!(push_pass(&state, unix_now()).await, 1);
    let old = tested_oid(&state, &head).await;
    fake.report_check_runs(
        &old,
        vec![check_run(1, REQUIRED_CHECK, &old, Some("success"))],
    );
    deliver_check_run(&state, GITHUB_REPOSITORY_ID, &old).await;

    push_main_change(
        &state,
        &owner_source,
        &remote,
        "README.md",
        "maintainer line\n",
    )
    .await;

    crate::use_cases::request_checks::reconcile_request_checks_once(
        &state,
        &mut Default::default(),
    )
    .await
    .unwrap();
    let member = bearer_header_for(MEMBER_SUBJECT, MEMBER_EMAIL);
    let view = checks(&state, REQUEST_ID, &member).await;
    assert_eq!(view["state"], "configuration-error");
    assert_eq!(view["message"], PRIVATE_CODE_CONFLICT_MESSAGE);
    assert_eq!(view["mergeability"]["status"], "ChecksConfigurationError");
    assert_eq!(
        merge(&state, REQUEST_ID).await.status(),
        StatusCode::CONFLICT
    );
    assert_eq!(push_pass(&state, unix_now()).await, 0);
    assert_eq!(
        fake.branch_head(&format!("scope/requests/{REQUEST_ID}")),
        Some(old)
    );
}

pub(super) async fn tested_commit(state: &AppState, head: &str) -> String {
    state
        .metadata
        .requests()
        .request_check_evaluation(REQUEST_ID, head)
        .await
        .unwrap()
        .unwrap()
        .tested_oid
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_contributors_push_reaches_github_only_after_a_maintainer_approves() {
    let (mut state, _owner_source) =
        test_state_with_mergeable_request("github-checks-contributor").await;
    let fake = connect_github(&mut state, &[REQUIRED_CHECK]).await;
    insert_member_user(&state).await;
    let member = bearer_header_for(MEMBER_SUBJECT, MEMBER_EMAIL);
    let public = bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL);
    let (source, remote, _server) = request_push_checkout(
        &state,
        "github-checks-contributor-push",
        PUBLIC_SUBJECT,
        PUBLIC_EMAIL,
    )
    .await;
    fs::create_dir_all(source.join(".github/workflows")).unwrap();
    push_change(
        &source,
        &remote,
        REQUEST_REF,
        ".github/workflows/ci.yml",
        "on: push\n",
        "change the workflow",
    )
    .unwrap();
    let head = git_head_oid(&source);
    let branch = format!("scope/requests/{REQUEST_ID}");
    let submitted = api_request(
        router(state.clone()),
        "POST",
        &format!("/v1/repos/{TEST_REPO_ID}/requests/{REQUEST_ID}/submit"),
        Some(&public),
        Some("{}"),
    )
    .await;
    assert_eq!(submitted.status(), StatusCode::OK);

    assert_eq!(push_pass(&state, unix_now()).await, 0);
    assert_eq!(fake.branch_head(&branch), None);
    let waiting = checks(&state, REQUEST_ID, &member).await;
    assert_eq!(waiting["state"], "awaiting-approval");
    assert_eq!(waiting["can_approve"], true);
    assert_eq!(waiting["github_push"]["state"], "awaiting_approval");
    assert_eq!(waiting["changes_github_workflows"], true);
    assert_eq!(
        state
            .metadata
            .requests()
            .request_check_evaluation(REQUEST_ID, &head)
            .await
            .unwrap()
            .unwrap()
            .changes_github_workflows,
        Some(true)
    );
    state
        .metadata
        .admin()
        .execute_for_tests(
            "UPDATE scope_request_check_evaluations SET changes_github_workflows = NULL",
        )
        .await
        .unwrap();
    assert_eq!(
        checks(&state, REQUEST_ID, &member).await["changes_github_workflows"],
        true
    );
    assert_eq!(
        state
            .metadata
            .requests()
            .request_check_evaluation(REQUEST_ID, &head)
            .await
            .unwrap()
            .unwrap()
            .changes_github_workflows,
        None
    );
    assert_eq!(
        crate::use_cases::request_checks::reconcile_request_checks_once(
            &state,
            &mut Default::default()
        )
        .await
        .unwrap(),
        1
    );
    assert_eq!(
        state
            .metadata
            .requests()
            .request_check_evaluation(REQUEST_ID, &head)
            .await
            .unwrap()
            .unwrap()
            .changes_github_workflows,
        Some(true)
    );
    let contributor_view = checks(&state, REQUEST_ID, &public).await;
    assert_eq!(contributor_view["can_approve"], false);
    assert_eq!(contributor_view["changes_github_workflows"], false);

    let approved = expect_json(
        api_request(
            router(state.clone()),
            "POST",
            &repo_request_checks_approve(TEST_REPO_OWNER, TEST_REPO_NAME, REQUEST_ID),
            Some(&member),
            Some(&reviewed_head_body(&state, REQUEST_ID).await),
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(approved["state"], "started");
    assert_eq!(approved["github_push"]["state"], "sending");
    assert_eq!(push_pass(&state, unix_now()).await, 1);
    let tested = tested_commit(&state, &head).await;
    assert_ne!(tested, head);
    assert_eq!(fake.branch_head(&branch), Some(tested));

    expect_json(
        api_request(
            router(state.clone()),
            "DELETE",
            &scope_api_contract::routes::repo_request(TEST_REPO_OWNER, TEST_REPO_NAME, REQUEST_ID),
            Some(&public),
            None,
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(push_pass(&state, unix_now()).await, 1);
    assert_eq!(fake.branch_head(&branch), None);
}
