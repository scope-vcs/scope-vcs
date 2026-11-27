use super::*;
use std::collections::BTreeMap;

#[test]
fn content_push_can_delete_former_rules_file() {
    let repo = published_repo_with_public_file("initial", "/.scope/RULES.md", "");
    let config = repo.repo_config.clone();
    let state = ContentPushState {
        change_version: repo.record.change_version,
        content_version: repo.record.content_version,
        policy: repo.policy.clone(),
        repo_config: config.clone(),
        live_files: repo.live_files.clone(),
        git_head: repo.git_head,
    };

    let accepted = accept_content_push(
        state,
        reviewed_update(
            "3333333333333333333333333333333333333333",
            "delete rules",
            vec![reviewed_change("/.scope/RULES.md", None)],
            Some(config.clone()),
            config,
        ),
    )
    .unwrap();

    assert_eq!(accepted.logical_commit.changes.len(), 1);
    assert_eq!(accepted.logical_commit.changes[0].new_content, None);
}

#[test]
fn request_merge_accepts_unchanged_tree_without_weakening_push_rules() {
    let repo = published_repo_with_public_file("initial", "/README.md", "hello");
    let config = repo.repo_config.clone();
    let state = ContentPushState {
        change_version: repo.record.change_version,
        content_version: repo.record.content_version,
        policy: repo.policy.clone(),
        repo_config: config.clone(),
        live_files: repo.live_files.clone(),
        git_head: repo.git_head,
    };
    let update = reviewed_update(
        "3333333333333333333333333333333333333333",
        "merge request",
        Vec::new(),
        Some(config.clone()),
        config,
    );

    assert!(accept_content_push(state.clone(), update.clone()).is_err());
    let accepted = accept_request_merge(
        state,
        update,
        RequestMergeOrigin::Canonical {
            request_id: "request-1".to_string(),
            request_head_oid: "2222222222222222222222222222222222222222".to_string(),
        },
    )
    .unwrap();
    assert_eq!(accepted.change_version, 2);
    assert_eq!(accepted.git_head.change_version, 2);
    assert_eq!(
        accepted.logical_commit.id,
        "rv_merge_3333333333333333333333333333333333333333"
    );
    assert_eq!(
        accepted.logical_commit.origin,
        LogicalCommitOrigin::PrivateRequestMerge {
            request_id: "request-1".to_string(),
            request_head_oid: "2222222222222222222222222222222222222222".to_string(),
        }
    );
    assert!(accepted.logical_commit.changes.is_empty());
}

#[test]
fn public_projection_keeps_historical_rules_changes() {
    let graph = graph(vec![commit(
        "rv1",
        "add rules",
        added("/.scope/RULES.md", ViewId::public(), ""),
    )]);

    let projection = project_graph(&graph, &[], &Views::builtin(), &ViewId::public());

    assert_eq!(projection.visible_paths(), vec!["/.scope/RULES.md"]);
}

#[test]
fn drifted_stored_labels_refuse_fast_content_updates_and_reconcile_on_reviewed_updates() {
    let mut repo = published_repo_with_public_file("initial", "/Secrets/key.txt", "old-secret");
    repo.repo_config.files.rules = vec![scope_domain::repo_config::RepoConfigFileRule {
        path: "/secrets/**".to_string(),
        view: ViewId::private(),
    }];
    repo.repo_config.validate().unwrap();
    let secret_path = ScopePath::parse("/Secrets/key.txt").unwrap();
    assert_eq!(
        repo.policy.label(&secret_path, repo.repo_config.views()),
        ViewId::public()
    );
    assert_eq!(
        repo.repo_config.label_for_path(&secret_path),
        ViewId::private()
    );

    let config = repo.repo_config.clone();
    let state = ContentPushState {
        change_version: repo.record.change_version,
        content_version: repo.record.content_version,
        policy: repo.policy.clone(),
        repo_config: config.clone(),
        live_files: repo.live_files.clone(),
        git_head: repo.git_head.clone(),
    };
    let update = reviewed_update(
        "3333333333333333333333333333333333333333",
        "rotate secret",
        vec![reviewed_change("/Secrets/key.txt", Some("new-secret"))],
        Some(config.clone()),
        config.clone(),
    );

    assert!(matches!(
        accept_content_push(state.clone(), update.clone()),
        Err(scope_domain::reviewed_updates::error::ReviewedUpdateError::PolicyDrift)
    ));
    assert!(matches!(
        accept_request_merge(
            state,
            update.clone(),
            RequestMergeOrigin::Canonical {
                request_id: "request-1".to_string(),
                request_head_oid: "2222222222222222222222222222222222222222".to_string(),
            },
        ),
        Err(scope_domain::reviewed_updates::error::ReviewedUpdateError::PolicyDrift)
    ));

    let mut pushed = repo.clone();
    apply_reviewed_update_to_repo(&mut pushed, update).unwrap();
    assert_eq!(
        pushed
            .policy
            .label(&secret_path, pushed.repo_config.views()),
        ViewId::private()
    );
    assert_eq!(
        pushed.graph.commits.last().unwrap().changes[0].label,
        ViewId::private()
    );
    assert!(
        project_graph(
            &pushed.graph,
            &pushed.visibility_change_sets,
            pushed.repo_config.views(),
            &ViewId::public(),
        )
        .visible_paths()
        .is_empty()
    );

    let mut saved = repo;
    let owner_id = saved.record.owner_user_id.clone();
    let changed = apply_reviewed_config_to_repo(
        &mut saved,
        ReviewedConfigUpdateInput {
            occurred_at_unix: 100,
            author_id: owner_id.clone(),
            config: config.clone(),
            open_requests_by_view: BTreeMap::new(),
        },
    )
    .unwrap();
    assert!(changed);
    assert_eq!(
        saved.policy.label(&secret_path, saved.repo_config.views()),
        ViewId::private()
    );
    assert!(
        project_graph(
            &saved.graph,
            &saved.visibility_change_sets,
            saved.repo_config.views(),
            &ViewId::public(),
        )
        .visible_paths()
        .is_empty()
    );
    let second_save = apply_reviewed_config_to_repo(
        &mut saved,
        ReviewedConfigUpdateInput {
            occurred_at_unix: 101,
            author_id: owner_id,
            config,
            open_requests_by_view: BTreeMap::new(),
        },
    )
    .unwrap();
    assert!(!second_save);
}
