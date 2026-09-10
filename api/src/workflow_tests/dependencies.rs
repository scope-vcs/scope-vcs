use super::*;
use scope_domain::{
    dependency_analysis::{AnalyzerOutput, DEPENDENCY_ANALYZER_VERSION, DependencyEdge},
    repo_config::RepoConfigVisibilityRule,
};
use scope_postgres::db::DependencyCompletion;

async fn dependency_fixture(label: &str) -> AppState {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let source = temp_git_repo(label);
    fs::create_dir_all(source.join("src")).unwrap();
    fs::create_dir_all(source.join("internal")).unwrap();
    fs::write(source.join("src/a.ts"), "export { value } from './b';\n").unwrap();
    fs::write(
        source.join("src/b.ts"),
        "import { secret } from '../internal/c';\nexport const value = secret;\n",
    )
    .unwrap();
    fs::write(source.join("internal/c.ts"), "export const secret = 42;\n").unwrap();
    run_git(Some(&source), &["add", "."], "stage dependency fixture").unwrap();
    commit_all(&source, "add direct dependency chain");
    let bare = clone_test_repo(&source, &format!("{label}-bare"), true);
    let mut config = repo_config(Visibility::Public);
    config.visibility.rules.push(RepoConfigVisibilityRule {
        path: "/internal/**".to_string(),
        visibility: ConfigVisibility::Private,
    });
    apply_first_push_from_staging_repo(&state, &bare, config).await;
    state
}

fn reader_output() -> AnalyzerOutput {
    AnalyzerOutput {
        analyzer_version: DEPENDENCY_ANALYZER_VERSION.to_string(),
        analyzed_files: vec!["src/a.ts".into(), "src/b.ts".into(), "internal/c.ts".into()],
        unsupported_files: Vec::new(),
        edges: vec![
            DependencyEdge {
                source_path: "src/a.ts".into(),
                target_path: "src/b.ts".into(),
                kind: "re-export".into(),
            },
            DependencyEdge {
                source_path: "src/b.ts".into(),
                target_path: "internal/c.ts".into(),
                kind: "import".into(),
            },
            DependencyEdge {
                source_path: "src/b.ts".into(),
                target_path: "internal/c.ts".into(),
                kind: "type-import".into(),
            },
        ],
        gaps: Vec::new(),
    }
}

async fn read_check(state: &AppState, authorization: Option<&str>) -> Response {
    api_request(
        router(state.clone()),
        "GET",
        "/v1/repos/owner/repo/dependencies",
        authorization,
        None,
    )
    .await
}

#[tokio::test]
async fn dependency_report_is_persisted_direct_only_and_maintainer_only() {
    let state = dependency_fixture("dependency-report-access").await;
    let member_id =
        scope_postgres::db::scope_user_id_for_auth_identity("clerk", "dependency_member");
    state
        .metadata
        .auth()
        .insert_user_for_tests(test_user(
            &member_id,
            "dependency-member",
            "dependency-member@example.com",
        ))
        .await
        .unwrap();
    state
        .metadata
        .repositories()
        .mutate_repository_for_tests(TEST_REPO_ID, |repo| {
            repo.members.push(test_repository_member(
                TEST_REPO_ID,
                &member_id,
                RepositoryMemberPermissions::default(),
            ));
        })
        .await
        .unwrap();

    let owner = bearer_header();
    let member = bearer_header_for("dependency_member", "dependency-member@example.com");
    let pending = read_check(&state, Some(&owner)).await;
    assert_eq!(pending.status(), StatusCode::OK);
    let pending = response_json(pending).await;
    assert_eq!(pending["status"], "Pending");
    assert_eq!(pending["report"], serde_json::Value::Null);

    let now = unix_now();
    let claim = state
        .metadata
        .jobs()
        .claim_dependency_analysis(
            "dependency-test",
            DEPENDENCY_ANALYZER_VERSION,
            now,
            60,
            &crate::persistence_ids::generate_persistence_id,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        state
            .metadata
            .jobs()
            .complete_dependency_analysis_claim(&claim, reader_output(), now + 1)
            .await
            .unwrap(),
        DependencyCompletion::Completed
    );
    let owner_result = response_json(read_check(&state, Some(&owner)).await).await;
    let member_result = response_json(read_check(&state, Some(&member)).await).await;
    assert_eq!(owner_result, member_result);
    assert_eq!(owner_result["status"], "Ready");
    assert_eq!(
        owner_result["report"]["commit_oid"],
        claim.git_head.head_oid
    );
    assert_eq!(owner_result["report"]["public_file_count"], 1);
    assert_eq!(
        owner_result["report"]["findings"],
        serde_json::json!([
            { "source_path": "src/b.ts", "target_path": "internal/c.ts" }
        ])
    );
    assert_eq!(
        read_check(&state, None).await.status(),
        StatusCode::UNAUTHORIZED
    );
    let outsider = bearer_header_for("dependency_outsider", "dependency-outsider@example.com");
    let outsider = read_check(&state, Some(&outsider)).await;
    assert_eq!(outsider.status(), StatusCode::NOT_FOUND);
    assert!(
        !response_json(outsider)
            .await
            .to_string()
            .contains("internal/c.ts")
    );
    assert!(
        state
            .metadata
            .jobs()
            .claim_dependency_analysis(
                "after-read",
                DEPENDENCY_ANALYZER_VERSION,
                now + 2,
                60,
                &crate::persistence_ids::generate_persistence_id,
            )
            .await
            .unwrap()
            .is_none(),
        "reading a report must not enqueue another analysis"
    );

    state
        .metadata
        .repositories()
        .mutate_repository_for_tests(TEST_REPO_ID, |repo| {
            repo.members.clear();
        })
        .await
        .unwrap();
    assert_eq!(
        read_check(&state, Some(&member)).await.status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn dependency_failed_refresh_retains_previous_results_and_commit_identity() {
    let state = dependency_fixture("dependency-report-refresh").await;
    let now = unix_now();
    let claim = state
        .metadata
        .jobs()
        .claim_dependency_analysis(
            "dependency-test",
            DEPENDENCY_ANALYZER_VERSION,
            now,
            60,
            &crate::persistence_ids::generate_persistence_id,
        )
        .await
        .unwrap()
        .unwrap();
    state
        .metadata
        .jobs()
        .complete_dependency_analysis_claim(&claim, reader_output(), now + 1)
        .await
        .unwrap();
    state
        .metadata
        .repositories()
        .mutate_repository_for_tests(TEST_REPO_ID, |repo| {
            repo.record.description = Some("Updated repository".into());
            repo.bump_change_version();
        })
        .await
        .unwrap();
    let owner = bearer_header();
    let updating = response_json(read_check(&state, Some(&owner)).await).await;
    assert_eq!(updating["status"], "Updating");
    assert_eq!(updating["report"]["public_file_count"], 1);
    let retry = state
        .metadata
        .jobs()
        .claim_dependency_analysis(
            "dependency-test",
            DEPENDENCY_ANALYZER_VERSION,
            now + 2,
            60,
            &crate::persistence_ids::generate_persistence_id,
        )
        .await
        .unwrap()
        .unwrap();
    assert!(retry.reusable_analysis.is_some());
    assert!(
        state
            .metadata
            .jobs()
            .fail_dependency_analysis_claim(&retry, "Dependency check could not finish", now + 3)
            .await
            .unwrap()
    );
    let failed = response_json(read_check(&state, Some(&owner)).await).await;
    assert_eq!(failed["status"], "Failed");
    assert_eq!(failed["report"], updating["report"]);
    assert_eq!(failed["error"], "Dependency check could not finish");
}
