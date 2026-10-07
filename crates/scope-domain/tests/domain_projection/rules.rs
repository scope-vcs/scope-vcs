use super::*;

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
        git_head: repo.git_head.clone(),
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
        git_head: repo.git_head.clone(),
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
