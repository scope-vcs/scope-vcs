use super::*;

#[test]
fn tree_sorts_folders_before_files_and_marks_workflows_as_private_control_paths() {
    let tree = ReviewTree::from_paths(
        &[
            "README.md".to_string(),
            ".scope/RULES.md".to_string(),
            ".scope/runs/test.yml".to_string(),
            "src/lib.rs".to_string(),
        ],
        &[],
    );

    let root_children = tree
        .node(tree.root_id())
        .children
        .iter()
        .map(|id| tree.node(*id).path.as_str())
        .collect::<Vec<_>>();
    assert_eq!(root_children, vec!["/.scope", "/src", "/README.md"]);
    assert!(tree.nodes().iter().any(|node| {
        node.path == "/.scope/runs/test.yml" && node.reserved && node.kind == ReviewNodeKind::File
    }));
    assert!(tree.nodes().iter().any(|node| {
        node.path == "/.scope/RULES.md" && node.reserved && node.kind == ReviewNodeKind::File
    }));
}

#[test]
fn change_statuses_follow_new_paths_and_preserve_literal_arrows_spaces_and_control_characters() {
    let paths = [
        " old -> new\t.rs\n".to_string(),
        "new.rs".to_string(),
        "unrelated.rs".to_string(),
    ];
    let tree = ReviewTree::from_paths(
        &paths,
        &[
            GitChangedPath {
                status: "M".to_string(),
                path: paths[0].clone(),
                previous_path: None,
            },
            GitChangedPath {
                status: "R100".to_string(),
                path: "new.rs".to_string(),
                previous_path: Some("old.rs".to_string()),
            },
        ],
    );
    let node = |path: &str| {
        tree.nodes()
            .iter()
            .find(|node| node.path == format!("/{path}"))
            .unwrap()
    };
    assert_eq!(node(&paths[0]).name, paths[0]);
    assert_eq!(node(&paths[0]).change_status.as_deref(), Some("M"));
    assert_eq!(node("new.rs").change_status.as_deref(), Some("R100"));
    assert_eq!(node("unrelated.rs").change_status, None);
}
