use super::*;
use scope_domain::views::ViewId;

const PUSH_ONLY_MEMBER_ID: &str = "user_push_only";
fn config_with_rules(default: ViewId, rules: &[(&str, ViewId)]) -> RepoConfig {
    let mut config = repo_config(default);
    config.files.rules = rules
        .iter()
        .map(
            |(path, visibility)| scope_domain::repo_config::RepoConfigFileRule {
                path: (*path).to_string(),
                view: visibility.clone(),
            },
        )
        .collect();
    config
}

async fn install_push_only_repo(state: &AppState, mut repo: Repository) {
    repo.collaboration.members.push(test_repository_member(
        TEST_REPO_ID,
        PUSH_ONLY_MEMBER_ID,
        member_permissions(true, false),
    ));
    replace_test_repo(state, repo).await;
}

async fn push_as_push_only_member(
    state: &AppState,
    changes: Vec<(&str, Option<&str>)>,
    config: RepoConfig,
) -> Result<scope_domain::repository::git::GitHead, crate::error::ApiError> {
    let mut update = receive_pack_update(state, changes);
    update.base_config_hash = repo_config_fingerprint(
        &find_repo(state, TEST_REPO_OWNER, TEST_REPO_NAME)
            .await?
            .repo_config,
    )?;
    update.config = config;
    persist_and_promote_test_update(state, update, PUSH_ONLY_MEMBER_ID).await
}

async fn rejected_config_push(
    state: &AppState,
    changes: Vec<(&str, Option<&str>)>,
    config: RepoConfig,
) -> Repository {
    let error = push_as_push_only_member(state, changes, config)
        .await
        .unwrap_err();
    assert_eq!(error.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        error.public_message(),
        "file visibility permission required"
    );
    find_repo(state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap()
}

#[tokio::test]
async fn push_only_member_cannot_publish_private_path_via_config() {
    let state = test_state_with_repo();
    let existing_config = config_with_rules(ViewId::private(), &[("/README.md", ViewId::public())]);
    let mut repo = repo_with_readme(&state);
    repo.repo_config = existing_config;
    repo.policy = Policy::new(ViewId::private());
    repo.policy
        .add_rule(LabelRule::public(ScopePath::parse("/README.md").unwrap()))
        .unwrap();
    install_push_only_repo(&state, repo).await;

    let config = config_with_rules(
        ViewId::private(),
        &[
            ("/README.md", ViewId::public()),
            ("/secret.txt", ViewId::public()),
        ],
    );

    let repo = rejected_config_push(&state, vec![("/secret.txt", Some("leak"))], config).await;
    assert!(
        !repo
            .live_files
            .contains_key(&ScopePath::parse("/secret.txt").unwrap())
    );
}

#[tokio::test]
async fn push_only_member_cannot_restore_stale_public_config_after_visibility_change() {
    let state = test_state_with_repo();
    let readme_path = ScopePath::parse("/README.md").unwrap();
    let mut repo = repo_with_readme(&state);
    scope_domain::reviewed_updates::config::apply_reviewed_config_to_repo(
        &mut repo,
        scope_domain::reviewed_updates::config::ReviewedConfigUpdateInput {
            author_id: test_owner_id(),
            occurred_at_unix: 10,
            config: config_with_rules(ViewId::public(), &[("/README.md", ViewId::private())]),
            open_requests_by_view: Default::default(),
        },
    )
    .unwrap();
    assert_eq!(
        repo.repo_config.label_for_path(&readme_path),
        ViewId::private()
    );
    install_push_only_repo(&state, repo).await;

    let repo = rejected_config_push(
        &state,
        vec![("/README.md", Some("member update"))],
        repo_config(ViewId::public()),
    )
    .await;
    assert_eq!(
        repo.policy.label(&readme_path, repo.repo_config.views()),
        ViewId::private()
    );
    assert_eq!(
        repo.repo_config.label_for_path(&readme_path),
        ViewId::private()
    );
}

#[tokio::test]
async fn content_push_with_unchanged_config_falls_back_and_reconciles_drifted_labels() {
    let state = test_state_with_repo();
    let mut repo = repo_with_readme(&state);
    let secret_path = ScopePath::parse("/Secrets/key.txt").unwrap();
    let blob = source_blob(&state, "old-secret");
    repo.live_files.insert(secret_path.clone(), blob.clone());
    repo.policy
        .add_rule(LabelRule::public(secret_path.clone()))
        .unwrap();
    repo.graph
        .commits
        .last_mut()
        .unwrap()
        .changes
        .push(scope_domain::projection::FileChange {
            path: secret_path.clone(),
            old_content: None,
            new_content: Some(blob),
            label: ViewId::public(),
        });
    repo.repo_config = config_with_rules(ViewId::public(), &[("/secrets/**", ViewId::private())]);
    replace_test_repo(&state, repo).await;

    let stored_repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    assert!(
        state
            .metadata
            .repositories()
            .has_drifted_visibility_labels(&stored_repo.record.id, &stored_repo.repo_config)
            .await
            .unwrap()
    );

    let mut update = receive_pack_update(&state, vec![("/README.md", Some("updated readme"))]);
    let stored_config = stored_repo.repo_config;
    update.previous_config = Some(stored_config.clone());
    update.base_config_hash = repo_config_fingerprint(&stored_config).unwrap();
    update.config = stored_config;
    persist_and_promote_test_update(&state, update, &test_owner_id())
        .await
        .unwrap();

    let updated = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    assert_eq!(
        updated
            .policy
            .label(&secret_path, updated.repo_config.views()),
        ViewId::private()
    );
    assert_eq!(
        updated.visibility_change_sets.last().unwrap().changes[0].path,
        secret_path
    );
    assert_eq!(
        updated.visibility_change_sets.last().unwrap().changes[0].new_label,
        ViewId::private()
    );
    assert!(
        !state
            .metadata
            .repositories()
            .has_drifted_visibility_labels(&updated.record.id, &updated.repo_config)
            .await
            .unwrap()
    );
}
