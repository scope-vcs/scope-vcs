use super::fake_github::{
    GITHUB_FULL_NAME, GITHUB_REPOSITORY_ID, connect_test_repository, github_api, github_json,
    github_state,
    issues::{github_user, issue, pull_request},
};
use super::*;
use crate::use_cases::github::issue_imports::import_due_github_issues;
use std::sync::atomic::Ordering;

const CREATED: u64 = 1_780_000_000;

async fn connected_state() -> (AppState, Arc<fake_github::FakeGitHub>) {
    let (state, fake) = github_state().await;
    connect_test_repository(&state, GITHUB_REPOSITORY_ID, GITHUB_FULL_NAME, 0).await;
    (state, fake)
}

async fn start(state: &AppState, bearer: &str, view: &str) -> Response {
    github_api(
        state,
        "POST",
        "/issue-import",
        bearer,
        Some(serde_json::json!({ "view": view })),
    )
    .await
}

async fn started(state: &AppState) -> serde_json::Value {
    expect_json(
        start(state, &bearer_header(), "private").await,
        StatusCode::OK,
    )
    .await["issue_import"]
        .clone()
}

async fn issue_import(state: &AppState) -> serde_json::Value {
    github_json(state, "GET", "").await["issue_import"].clone()
}

async fn import_pass(state: &AppState, now: u64) -> usize {
    import_due_github_issues(state, now).await.unwrap()
}

async fn request_json(state: &AppState, issue_number: u64, path: &str) -> serde_json::Value {
    let request = state
        .metadata
        .requests()
        .request_by_github_issue_number(TEST_REPO_ID, issue_number)
        .await
        .unwrap()
        .unwrap();
    let read = api_request(
        router(state.clone()),
        "GET",
        &format!(
            "/v1/repos/{TEST_REPO_OWNER}/{TEST_REPO_NAME}/requests/{}/{path}",
            request.id
        ),
        Some(&bearer_header()),
        None,
    )
    .await;
    expect_json(read, StatusCode::OK).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_maintainer_imports_open_issues_once_as_requests_with_their_comments() {
    let (state, fake) = connected_state().await;
    *fake.issues.open.lock().unwrap() = vec![
        issue(1, "Login times out", github_user(7, "tomasz-k"), CREATED),
        pull_request(2, CREATED + 10),
        issue(4, " ", github_user(7, "tomasz-k"), CREATED + 20),
    ];
    fake.issues.report_comments(
        1,
        vec![
            (32, serde_json::Value::Null, "Same here.", CREATED + 200),
            (
                31,
                github_user(9, "ines-r"),
                "Which version?",
                CREATED + 100,
            ),
        ],
    );
    assert_eq!(
        github_json(&state, "GET", "").await["issue_import_default_view"],
        "private"
    );

    let stranger = bearer_header_for("user_github_stranger", "stranger@example.com");
    let denied = start(&state, &stranger, "private").await;
    assert!(denied.status().is_client_error(), "{}", denied.status());
    let unknown_view = start(&state, &bearer_header(), "archive").await;
    assert_eq!(unknown_view.status(), StatusCode::BAD_REQUEST);
    let queued = started(&state).await;
    assert_eq!(
        (&queued["state"], &queued["view"]),
        (&"queued".into(), &"private".into())
    );
    let working = start(&state, &bearer_header(), "public").await;
    assert_eq!(working.status(), StatusCode::CONFLICT);

    assert_eq!(import_pass(&state, unix_now()).await, 1);
    let imported = issue_import(&state).await;
    let blank_title_skipped = serde_json::json!([
        { "number": 4, "reason": "Scope cannot hold this issue: title is required." }
    ]);
    assert_eq!(
        (
            &imported["state"],
            &imported["imported_count"],
            &imported["already_imported_count"],
            &imported["skipped_issues"],
            &imported["error"],
        ),
        (
            &"succeeded".into(),
            &1.into(),
            &0.into(),
            &blank_title_skipped,
            &serde_json::Value::Null,
        )
    );

    let detail = request_json(&state, 1, "state").await["detail"].clone();
    assert_eq!(detail["asker"]["login"], "tomasz-k");
    let request = &detail["request"];
    assert_eq!(request["title"], "Login times out");
    assert_eq!(request["view"], "private");
    assert_eq!(request["ask"]["handle"], "gh-1");
    assert_eq!(request["ask"]["text_markdown"], "Body of issue 1.");
    assert_eq!(request["ask"]["asked_at_unix"], CREATED);
    assert_eq!(
        request["ask"]["source_url"],
        format!("https://github.com/{GITHUB_FULL_NAME}/issues/1")
    );
    let timeline = request_json(&state, 1, "timeline").await;
    let mut comments = timeline["discussions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|discussion| {
            (
                discussion["opened_position"].as_u64().unwrap(),
                discussion["author"]["account"]["login"].as_str().unwrap(),
                discussion["body_markdown"].as_str().unwrap(),
                discussion["created_at_unix"].as_u64().unwrap(),
                discussion["anchor"].is_null(),
            )
        })
        .collect::<Vec<_>>();
    comments.sort();
    assert_eq!(
        comments
            .iter()
            .map(|(_, login, body, created, unanchored)| (*login, *body, *created, *unanchored))
            .collect::<Vec<_>>(),
        [
            ("ines-r", "Which version?", CREATED + 100, true),
            ("ghost", "Same here.", CREATED + 200, true),
        ]
    );
    for not_imported in [2, 4] {
        assert_eq!(
            state
                .metadata
                .requests()
                .request_by_github_issue_number(TEST_REPO_ID, not_imported)
                .await
                .unwrap(),
            None
        );
    }

    {
        let mut open = fake.issues.open.lock().unwrap();
        open[0]["title"] = "Login times out everywhere".into();
        open.push(issue(
            5,
            "Export drops the header",
            github_user(9, "ines-r"),
            CREATED + 30,
        ));
    }
    let comment_reads = fake.issues.comment_reads.load(Ordering::SeqCst);
    started(&state).await;
    assert_eq!(import_pass(&state, unix_now()).await, 1);
    let again = issue_import(&state).await;
    assert_eq!(
        (
            &again["state"],
            &again["imported_count"],
            &again["already_imported_count"],
            &again["skipped_issues"],
        ),
        (
            &"succeeded".into(),
            &1.into(),
            &1.into(),
            &blank_title_skipped,
        )
    );
    assert_eq!(
        fake.issues.comment_reads.load(Ordering::SeqCst),
        comment_reads + 2,
        "only the new issue and the one that was skipped are read again"
    );
    let unchanged = request_json(&state, 1, "state").await;
    assert_eq!(unchanged["detail"]["request"]["title"], "Login times out");
    assert_eq!(
        request_json(&state, 5, "state").await["detail"]["request"]["title"],
        "Export drops the header"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_import_that_fails_part_way_shows_why_and_resumes_without_counting_twice() {
    let (state, fake) = connected_state().await;
    *fake.issues.open.lock().unwrap() = (1..=3)
        .map(|number| {
            issue(
                number,
                "An issue",
                github_user(7, "tomasz-k"),
                CREATED + number,
            )
        })
        .collect();
    *fake.issues.comments_unavailable_for.lock().unwrap() = Some(2);
    started(&state).await;
    let now = unix_now();

    assert_eq!(import_pass(&state, now).await, 1);
    let retrying = issue_import(&state).await;
    assert_eq!(
        (&retrying["state"], &retrying["imported_count"]),
        (&"queued".into(), &1.into())
    );
    let error = retrying["error"].as_str().unwrap();
    assert!(error.contains("502 Bad Gateway"), "{error}");
    assert_eq!(import_pass(&state, now).await, 0);

    *fake.issues.comments_unavailable_for.lock().unwrap() = None;
    assert_eq!(import_pass(&state, now + 31).await, 1);
    let finished = issue_import(&state).await;
    assert_eq!(
        (
            &finished["state"],
            &finished["imported_count"],
            &finished["already_imported_count"],
            &finished["error"],
        ),
        (
            &"succeeded".into(),
            &3.into(),
            &0.into(),
            &serde_json::Value::Null,
        )
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn issues_are_read_page_by_page_and_a_rate_limit_waits_for_its_reset() {
    let (state, fake) = connected_state().await;
    *fake.issues.open.lock().unwrap() = (1..=100)
        .map(|number| pull_request(number, CREATED + number))
        .chain([issue(
            101,
            "The only issue",
            github_user(7, "tomasz-k"),
            CREATED + 101,
        )])
        .collect();
    let now = unix_now();
    let reset = now + 600;
    *fake.issues.list_rate_limited_until.lock().unwrap() = Some(reset);
    started(&state).await;

    assert_eq!(import_pass(&state, now).await, 1);
    let limited = issue_import(&state).await;
    assert_eq!(limited["state"], "queued");
    assert_eq!(limited["error"], crate::github::RATE_LIMIT_MESSAGE);
    assert_eq!(import_pass(&state, reset - 1).await, 0);

    *fake.issues.list_rate_limited_until.lock().unwrap() = None;
    fake.client_clock_unix.store(reset, Ordering::SeqCst);
    let list_reads = fake.issues.list_reads.load(Ordering::SeqCst);
    assert_eq!(import_pass(&state, reset).await, 1);
    assert_eq!(
        fake.issues.list_reads.load(Ordering::SeqCst),
        list_reads + 3,
        "the first page is read again before the second, to notice issues that moved"
    );
    let finished = issue_import(&state).await;
    assert_eq!(
        (&finished["state"], &finished["imported_count"]),
        (&"succeeded".into(), &1.into())
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_installation_that_cannot_read_issues_fails_the_import_with_what_to_grant() {
    let (state, fake) = connected_state().await;
    *fake.issues.open.lock().unwrap() =
        vec![issue(1, "An issue", github_user(7, "tomasz-k"), CREATED)];
    let granted = fake
        .check_sources
        .permissions
        .lock()
        .unwrap()
        .as_object_mut()
        .unwrap()
        .remove("issues")
        .unwrap();
    started(&state).await;

    assert_eq!(import_pass(&state, unix_now()).await, 1);
    let failed = issue_import(&state).await;
    assert_eq!(failed["state"], "failed");
    assert_eq!(
        failed["error"],
        "The Scope GitHub App is missing issues (read). Accept its requested permissions in \
         GitHub's installation settings. Then import again."
    );
    assert_eq!(fake.issues.list_reads.load(Ordering::SeqCst), 0);

    fake.check_sources.permissions.lock().unwrap()["issues"] = granted;
    assert_eq!(started(&state).await["state"], "queued");
    assert_eq!(import_pass(&state, unix_now()).await, 1);
    assert_eq!(issue_import(&state).await["imported_count"], 1);
}

fn issues(numbers: std::ops::RangeInclusive<u64>) -> Vec<serde_json::Value> {
    numbers
        .map(|number| {
            issue(
                number,
                "An issue",
                github_user(7, "tomasz-k"),
                CREATED + number,
            )
        })
        .collect()
}

async fn import_pass_changed_at_issue(
    state: &AppState,
    fake: &fake_github::FakeGitHub,
    issue_number: u64,
    change: impl AsyncFnOnce(),
) {
    let held = fake.issues.hold_comments_of(issue_number);
    let pass = tokio::spawn({
        let state = state.clone();
        async move { import_pass(&state, unix_now()).await }
    });
    held.arrived.notified().await;
    change().await;
    held.released.notify_one();
    assert_eq!(pass.await.unwrap(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_issue_github_no_longer_has_is_listed_as_skipped_and_the_rest_are_imported() {
    let (state, fake) = connected_state().await;
    *fake.issues.open.lock().unwrap() = issues(1..=3);
    *fake.issues.comments_missing_for.lock().unwrap() = Some(2);
    started(&state).await;

    assert_eq!(import_pass(&state, unix_now()).await, 1);
    let finished = issue_import(&state).await;
    assert_eq!(
        (
            &finished["state"],
            &finished["imported_count"],
            &finished["skipped_issues"],
        ),
        (
            &"succeeded".into(),
            &2.into(),
            &serde_json::json!([
                { "number": 2, "reason": "GitHub no longer has this issue." }
            ]),
        )
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn losing_the_github_repository_part_way_fails_the_import_instead_of_finishing_short() {
    let (state, fake) = connected_state().await;
    *fake.issues.open.lock().unwrap() = issues(1..=3);
    started(&state).await;

    import_pass_changed_at_issue(&state, &fake, 2, async || {
        fake.issues.repository_gone.store(true, Ordering::SeqCst);
    })
    .await;
    let failed = issue_import(&state).await;
    assert_eq!(
        (
            &failed["state"],
            &failed["error"],
            &failed["imported_count"],
            &failed["skipped_issues"],
        ),
        (
            &"failed".into(),
            &"The Scope GitHub App can no longer reach this GitHub repository.".into(),
            &1.into(),
            &serde_json::json!([]),
        )
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn removing_the_chosen_view_part_way_fails_the_import_and_asks_for_another_view() {
    let (mut state, head, _source) =
        super::custom_views::fixture("issue-import-view-removed").await;
    let fake = fake_github::FakeGitHub::install(&mut state).await;
    connect_test_repository(&state, GITHUB_REPOSITORY_ID, GITHUB_FULL_NAME, 0).await;
    *fake.issues.open.lock().unwrap() = issues(1..=2);
    let queued = expect_json(
        start(&state, &bearer_header(), "agent").await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(queued["issue_import"]["view"], "agent");
    let reassigned = api_request(
        router(state.clone()),
        "PATCH",
        &format!(
            "/v1/repos/owner/repo/members/{}",
            super::custom_views::member_id()
        ),
        Some(&bearer_header()),
        Some(r#"{"permissions":{"can_push":false,"can_change_file_visibility":false,"view":"public"}}"#),
    )
    .await;
    expect_json(reassigned, StatusCode::OK).await;
    let unlabelled = super::custom_views::config_with(
        vec![super::custom_views::custom_view(
            "agent",
            "Agent",
            &["public"],
        )],
        &[],
    );
    expect_json(
        super::custom_views::push_config(&state, &head, &unlabelled).await,
        StatusCode::OK,
    )
    .await;

    import_pass_changed_at_issue(&state, &fake, 1, async || {
        let without_agent = super::custom_views::config_with(Vec::new(), &[]);
        expect_json(
            super::custom_views::push_config(&state, &head, &without_agent).await,
            StatusCode::OK,
        )
        .await;
    })
    .await;
    let failed = issue_import(&state).await;
    assert_eq!(
        (
            &failed["state"],
            &failed["error"],
            &failed["imported_count"],
            &failed["skipped_issues"],
        ),
        (
            &"failed".into(),
            &"This repository no longer has the agent view. Choose another view and import again."
                .into(),
            &0.into(),
            &serde_json::json!([]),
        )
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn comments_past_the_ones_read_leave_a_line_on_the_last_one_imported() {
    let (state, fake) = connected_state().await;
    *fake.issues.open.lock().unwrap() = issues(1..=1);
    fake.issues.report_comments(
        1,
        (1..=1000)
            .map(|id| {
                let body = match id {
                    1 => "Which version?",
                    999 => "Still broken on 2.14.1.",
                    _ => " ",
                };
                (id, github_user(9, "ines-r"), body, CREATED + id)
            })
            .collect(),
    );
    started(&state).await;

    assert_eq!(import_pass(&state, unix_now()).await, 1);
    assert_eq!(issue_import(&state).await["imported_count"], 1);
    let timeline = request_json(&state, 1, "timeline").await;
    let mut comments = timeline["discussions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|discussion| {
            (
                discussion["opened_position"].as_u64().unwrap(),
                discussion["body_markdown"].as_str().unwrap(),
            )
        })
        .collect::<Vec<_>>();
    comments.sort();
    assert_eq!(
        comments.iter().map(|(_, body)| *body).collect::<Vec<_>>(),
        [
            "Which version?".to_string(),
            format!(
                "Still broken on 2.14.1.\n\n_Later comments were not imported. [Read the rest on \
                 GitHub](https://github.com/{GITHUB_FULL_NAME}/issues/1)._"
            ),
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_issue_that_moves_to_an_earlier_page_when_another_closes_is_still_imported() {
    let (state, fake) = connected_state().await;
    *fake.issues.open.lock().unwrap() = (1..=99)
        .map(|number| pull_request(number, CREATED + number))
        .chain(issues(100..=102))
        .collect();
    started(&state).await;

    import_pass_changed_at_issue(&state, &fake, 100, async || {
        fake.issues.open.lock().unwrap().remove(0);
    })
    .await;
    let finished = issue_import(&state).await;
    assert_eq!(
        (
            &finished["state"],
            &finished["imported_count"],
            &finished["already_imported_count"],
        ),
        (&"succeeded".into(), &3.into(), &0.into())
    );
    for imported in 100..=102 {
        assert!(
            state
                .metadata
                .requests()
                .request_by_github_issue_number(TEST_REPO_ID, imported)
                .await
                .unwrap()
                .is_some(),
            "issue {imported}"
        );
    }
}
