use super::*;
use scope_domain::views::ViewId;

const REQUEST_FILES: &str = "/v1/repos/owner/repo/files?request=req_full";

async fn file_paths(state: &AppState, route: &str, bearer: &str) -> Vec<String> {
    let response = api_request(router(state.clone()), "GET", route, Some(bearer), None).await;
    assert_eq!(response.status(), StatusCode::OK);
    response_json(response)
        .await
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file["path"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_code_reads_stay_inside_the_readers_view() {
    let (state, source, _head) =
        super::super::push_intent_completion::published_git_fixture("request-code-reads").await;
    cache_test_jwks(&state);
    state
        .metadata
        .auth()
        .insert_user_for_tests(test_user(public_user_id(), "public", PUBLIC_EMAIL))
        .await
        .unwrap();
    state
        .metadata
        .repositories()
        .mutate_repository_for_tests(TEST_REPO_ID, |repo| {
            repo.repo_config
                .files
                .rules
                .push(scope_domain::repo_config::RepoConfigFileRule {
                    path: "/secret/**".to_string(),
                    view: ViewId::private(),
                });
            repo.repo_config.validate().unwrap();
            repo.bump_content_version();
        })
        .await
        .unwrap();
    let main_head = git_stdout_text(&source, &["rev-parse", "HEAD"], "read main head")
        .unwrap()
        .trim()
        .to_string();
    state
        .metadata
        .requests()
        .start_request(StartRequestInput {
            id: "req_full".into(),
            repo_id: TEST_REPO_ID.into(),
            name: "full".into(),
            author_user_id: test_owner_id(),
            author_role: RequestActorRole::Owner,
            author_view: ViewId::private(),
            view: ViewId::private(),
            base: scope_domain::requests::RequestBase::Main {
                review_base_oid: main_head.clone(),
            },
            main_ancestor_oid: main_head,
            title: None,
            event_id: "start_full".into(),
            now_unix: 3,
        })
        .await
        .unwrap();

    let bearer = bearer_header();
    let (origin, _server) = spawn_test_server(&state).await;
    let remote = format!("{origin}/git/private/{TEST_REPO_ID}");
    configure_bearer_header(&source, &remote, &bearer);
    fs::create_dir_all(source.join("secret")).unwrap();
    fs::write(source.join("secret/key.txt"), "private value\n").unwrap();
    fs::write(source.join("notes.md"), "request notes\n").unwrap();
    run_git(Some(&source), &["add", "-A"], "stage request files").unwrap();
    commit_all(&source, "add request files");
    run_git(
        Some(&source),
        &["push", &remote, "HEAD:refs/heads/full"],
        "push request files",
    )
    .unwrap();

    let full = file_paths(&state, REQUEST_FILES, &bearer).await;
    assert!(full.contains(&"/secret/key.txt".to_string()), "{full:?}");
    assert!(full.contains(&"/notes.md".to_string()), "{full:?}");

    let public = file_paths(&state, &format!("{REQUEST_FILES}&view=public"), &bearer).await;
    assert!(public.contains(&"/notes.md".to_string()), "{public:?}");
    assert!(
        !public.iter().any(|path| path.starts_with("/secret/")),
        "{public:?}"
    );

    let content = "/v1/repos/owner/repo/files/content?request=req_full";
    let notes = api_request(
        router(state.clone()),
        "GET",
        &format!("{content}&path=notes.md&view=public"),
        Some(&bearer),
        None,
    )
    .await;
    assert_eq!(notes.status(), StatusCode::OK);
    assert_eq!(
        response_json(notes).await["content"]["text"],
        "request notes\n"
    );
    let hidden = api_request(
        router(state.clone()),
        "GET",
        &format!("{content}&path=secret/key.txt&view=public"),
        Some(&bearer),
        None,
    )
    .await;
    assert_eq!(hidden.status(), StatusCode::NOT_FOUND);

    let stranger = api_request(router(state.clone()), "GET", REQUEST_FILES, None, None).await;
    assert_eq!(stranger.status(), StatusCode::NOT_FOUND);

    let conflicting = api_request(
        router(state),
        "GET",
        &format!("{REQUEST_FILES}&update=any"),
        Some(&bearer),
        None,
    )
    .await;
    assert_eq!(conflicting.status(), StatusCode::BAD_REQUEST);
}
