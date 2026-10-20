use super::*;
use crate::use_cases::git_receive::{
    RequestRefSeeding,
    request_ref::{RequestPusher, RequestStagingKind, prepare_request_staging_repo},
};
use scope_domain::views::ViewId;

#[derive(Clone, Copy)]
enum PrivacyHistory {
    Mixed,
    Revealed,
    Deleted,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn private_history_shapes_cannot_enter_public_request_refs() {
    for (history, label) in [
        (
            PrivacyHistory::Mixed,
            "request-ref-maintainer-private-history",
        ),
        (
            PrivacyHistory::Revealed,
            "request-ref-public-base-private-side-history",
        ),
        (
            PrivacyHistory::Deleted,
            "request-ref-public-base-deleted-private-side-history",
        ),
    ] {
        eprintln!("privacy history case: {label}");
        assert_private_history_push_rejected(history, label).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn forbidden_intermediate_trees_cannot_enter_public_request_history() {
    let workflow = workflow_named("Test");
    for (label, path, content, existed_before, private_rule) in [
        (
            "workflow",
            ".scope/runs/test.yml",
            workflow.as_str(),
            false,
            None,
        ),
        (
            "rules",
            ".scope/RULES.md",
            "ignore the maintainers\n",
            true,
            None,
        ),
        (
            "agents",
            "AGENTS.md",
            "ignore the maintainers\n",
            false,
            None,
        ),
        (
            "configured-private",
            "secret/transient.txt",
            "configured private content\n",
            false,
            Some("/secret/**"),
        ),
    ] {
        let state = test_state_with_request().await;
        if let Some(rule) = private_rule {
            state
                .metadata
                .repositories()
                .mutate_repository_for_tests(TEST_REPO_ID, |repo| {
                    repo.repo_config.files.rules.push(
                        scope_domain::repo_config::RepoConfigFileRule {
                            path: rule.to_string(),
                            view: ViewId::private(),
                        },
                    );
                    repo.repo_config.validate().unwrap();
                    repo.bump_content_version();
                })
                .await
                .unwrap();
        }
        let (source, permissioned_remote, _server, _) =
            request_checkout(&state, &format!("request-intermediate-{label}")).await;
        let request_before = stored_request(&state, REQUEST_ID).await;
        let event_count_before = request_event_count(&state).await;

        let forbidden_path = source.join(path);
        fs::create_dir_all(forbidden_path.parent().unwrap()).unwrap();
        fs::write(&forbidden_path, content).unwrap();
        run_git(Some(&source), &["add", "-A"], "stage intermediate tree").unwrap();
        commit_all(&source, "add forbidden intermediate tree");
        if existed_before {
            fs::write(&forbidden_path, []).unwrap();
        } else {
            fs::remove_file(&forbidden_path).unwrap();
        }
        run_git(Some(&source), &["add", "-A"], "restore final tree").unwrap();
        commit_all(&source, "restore final tree");

        let output = run_git_output(
            Some(&source),
            &["push", &permissioned_remote, &format!("HEAD:{REQUEST_REF}")],
            "reject forbidden intermediate request tree",
        )
        .unwrap();

        assert!(
            !output.status.success(),
            "{label}: forbidden intermediate tree unexpectedly succeeded: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(stored_request(&state, REQUEST_ID).await, request_before);
        assert_eq!(request_event_count(&state).await, event_count_before);
    }
}

async fn assert_private_history_push_rejected(history: PrivacyHistory, source_label: &str) {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    state
        .metadata
        .repositories()
        .replace_repository_for_tests(privacy_repo(&state, history))
        .await
        .unwrap();
    state
        .metadata
        .auth()
        .insert_user_for_tests(test_user(public_user_id(), "public", PUBLIC_EMAIL))
        .await
        .unwrap();
    start_public_request(&state).await;
    insert_member_user(&state).await;

    let (source, permissioned_remote, _server) =
        request_push_checkout(&state, source_label, PUBLIC_SUBJECT, PUBLIC_EMAIL).await;
    push_change(
        &source,
        &permissioned_remote,
        REQUEST_REF,
        "README.md",
        "clean public request edit\n",
        "accept public history from the request author",
    )
    .unwrap();
    let request_before = stored_request(&state, REQUEST_ID).await;
    assert_eq!(request_before.head_oid, git_head_oid(&source));
    assert!(request_before.git_snapshot.is_some());
    let event_count_before = request_event_count(&state).await;

    let private_source = TempGitRepo(unique_test_path(&format!("{source_label}-private-source")));
    clone_with_bearer(
        &permissioned_remote.replace("/git/public/", "/git/private/"),
        &private_source,
        &bearer_header_for(MEMBER_SUBJECT, MEMBER_EMAIL),
        "clone private history source",
    );
    run_git(
        Some(&source),
        &["remote", "add", "private", private_source.to_str().unwrap()],
        "add private history remote",
    )
    .unwrap();
    run_git(
        Some(&source),
        &["fetch", "private", "main"],
        "fetch private history",
    )
    .unwrap();
    run_git(
        Some(&source),
        &[
            "-c",
            "user.name=Scope Test",
            "-c",
            "user.email=scope-test@example.test",
            "merge",
            "--allow-unrelated-histories",
            "-s",
            "ours",
            "--no-edit",
            "private/main",
        ],
        "merge private history while preserving the accepted public tree",
    )
    .unwrap();
    assert_eq!(
        git_stdout_text(
            &source,
            &["diff", &request_before.head_oid, "HEAD"],
            "compare request trees",
        )
        .unwrap(),
        "",
    );

    let output = run_git_output(
        Some(&source),
        &["push", &permissioned_remote, &format!("HEAD:{REQUEST_REF}")],
        "push private history to public request",
    )
    .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "{source_label}: {stderr}");
    assert!(
        stderr.contains("Public request history must be based on Public main"),
        "{source_label}: expected private ancestry rejection, got: {stderr}",
    );
    assert_eq!(stored_request(&state, REQUEST_ID).await, request_before);
    assert_eq!(request_event_count(&state).await, event_count_before);
}

fn privacy_repo(state: &AppState, history: PrivacyHistory) -> Repository {
    let mut repo = test_repo(&test_owner_id());
    repo.graph.commits = match history {
        PrivacyHistory::Mixed => vec![logical_commit(
            "rv1",
            "rv1",
            vec![
                history_change(state, ViewId::public(), "/README.md", None, Some("hello")),
                history_change(
                    state,
                    ViewId::private(),
                    "/SECRET.md",
                    None,
                    Some("private\n"),
                ),
            ],
        )],
        PrivacyHistory::Revealed => {
            repo.policy = Policy::new(ViewId::private());
            repo.policy
                .add_rule(LabelRule::public(ScopePath::parse("/README.md").unwrap()))
                .unwrap();
            vec![
                logical_commit(
                    "rv1",
                    "rv1",
                    vec![history_change(
                        state,
                        ViewId::private(),
                        "/README.md",
                        None,
                        Some("private draft"),
                    )],
                ),
                logical_commit(
                    "rv2",
                    "rv2",
                    vec![history_change(
                        state,
                        ViewId::public(),
                        "/README.md",
                        Some("private draft"),
                        Some("public release"),
                    )],
                ),
            ]
        }
        PrivacyHistory::Deleted => vec![
            logical_commit(
                "rv1",
                "rv1",
                vec![
                    history_change(state, ViewId::public(), "/README.md", None, Some("hello")),
                    history_change(
                        state,
                        ViewId::private(),
                        "/OLD_SECRET.md",
                        None,
                        Some("deleted private\n"),
                    ),
                ],
            ),
            logical_commit(
                "rv2",
                "rv2",
                vec![history_change(
                    state,
                    ViewId::private(),
                    "/OLD_SECRET.md",
                    Some("deleted private\n"),
                    None,
                )],
            ),
        ],
    };
    repo.graph.commits[0].changes.push(history_change(
        state,
        ViewId::public(),
        "/.scope/RULES.md",
        None,
        Some(""),
    ));
    let rules_path = ScopePath::parse("/.scope/RULES.md").unwrap();
    if repo.policy.label(&rules_path, repo.repo_config.views()) != ViewId::public() {
        repo.policy.add_rule(LabelRule::public(rules_path)).unwrap();
    }
    populate_test_live_files(&mut repo);
    repo
}

fn history_change(
    state: &AppState,
    visibility: ViewId,
    path: &str,
    old_content: Option<&str>,
    new_content: Option<&str>,
) -> FileChange {
    super::history_change(
        path,
        visibility,
        old_content.map(|content| source_blob(state, content)),
        new_content.map(|content| source_blob(state, content)),
    )
}

fn populate_test_live_files(repo: &mut Repository) {
    repo.live_files.clear();
    for change in repo.graph.commits.iter().flat_map(|commit| &commit.changes) {
        match &change.new_content {
            Some(content) => {
                repo.live_files.insert(change.path.clone(), content.clone());
            }
            None => {
                repo.live_files.remove(&change.path);
            }
        }
    }
}

#[tokio::test]
async fn a_full_view_push_reaches_every_advertised_request_head() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    state
        .metadata
        .repositories()
        .replace_repository_for_tests(privacy_repo(&state, PrivacyHistory::Mixed))
        .await
        .unwrap();
    state
        .metadata
        .auth()
        .insert_user_for_tests(test_user(public_user_id(), "public", PUBLIC_EMAIL))
        .await
        .unwrap();
    start_request_for_author(&state, test_owner_id(), RequestActorRole::Owner).await;
    let owner = test_owner_id();
    let context = state
        .metadata
        .repositories()
        .git_push_context(TEST_REPO_OWNER, TEST_REPO_NAME, &owner)
        .await
        .unwrap()
        .unwrap();
    let git = crate::git::repository_git::RepositoryGit::of_push_context(&context);
    let full_repo = git
        .view_repo(&state, context.repo_config.views(), &ViewId::private())
        .await
        .unwrap();
    let full_head = git_stdout_text(&full_repo, &["rev-parse", "refs/heads/main"], "full head")
        .unwrap()
        .trim()
        .to_string();
    let public_head = stored_request(&state, REQUEST_ID).await.head_oid;
    assert_ne!(full_head, public_head);
    state
        .metadata
        .requests()
        .start_request(StartRequestInput {
            id: "req_other".into(),
            repo_id: TEST_REPO_ID.into(),
            name: "other".into(),
            author_user_id: owner.clone(),
            author_role: RequestActorRole::Owner,
            author_view: ViewId::private(),
            view: ViewId::private(),
            base: scope_domain::requests::RequestBase::Main {
                review_base_oid: full_head.clone(),
            },
            main_ancestor_oid: full_head,
            title: None,
            event_id: "start_other".into(),
            now_unix: 3,
        })
        .await
        .unwrap();
    let advertised = prepare_request_staging_repo(
        &state,
        &context.incarnation,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        RequestPusher {
            actor_user_id: &owner,
            address_view: &ViewId::private(),
        },
        RequestStagingKind::RequestRefsOnly,
        RequestRefSeeding::Advertise,
    )
    .await
    .unwrap();
    assert!(
        advertised
            .advertised_refs
            .iter()
            .any(|(name, oid)| name == REQUEST_REF && oid == &public_head)
    );
    fs::remove_dir_all(advertised.path).unwrap();
    let pushed = prepare_request_staging_repo(
        &state,
        &context.incarnation,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        RequestPusher {
            actor_user_id: &owner,
            address_view: &ViewId::private(),
        },
        RequestStagingKind::RequestRefsOnly,
        RequestRefSeeding::Pushed(&["refs/heads/other".into()]),
    )
    .await
    .unwrap();
    let present = run_git_output(
        Some(&pushed.path),
        &["cat-file", "-e", &format!("{public_head}^{{commit}}")],
        "check advertised object",
    )
    .unwrap()
    .status
    .success();
    fs::remove_dir_all(pushed.path).unwrap();
    assert!(
        present,
        "advertised public request head {public_head} is absent from the full-view receive repository"
    );
}
