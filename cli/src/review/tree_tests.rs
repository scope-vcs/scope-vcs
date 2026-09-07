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
fn tree_maps_rename_status_to_new_path() {
    let tree = ReviewTree::from_paths(
        &["new.rs".to_string()],
        &[GitChangedPath {
            status: "R100".to_string(),
            path: "new.rs".to_string(),
            previous_path: Some("old.rs".to_string()),
        }],
    );

    let file = tree
        .nodes()
        .iter()
        .find(|node| node.path == "/new.rs")
        .unwrap();
    assert_eq!(file.change_status.as_deref(), Some("R100"));
}

#[test]
fn change_paths_preserve_literal_arrows_spaces_and_control_characters() {
    let paths = [" old -> new\t.rs\n".to_string(), "unrelated.rs".to_string()];
    let tree = ReviewTree::from_paths(
        &paths,
        &[GitChangedPath {
            status: "M".to_string(),
            path: paths[0].clone(),
            previous_path: None,
        }],
    );
    let file = tree
        .nodes()
        .iter()
        .find(|node| node.path == format!("/{}", paths[0]))
        .unwrap();
    assert_eq!(file.name, paths[0]);
    assert_eq!(file.change_status.as_deref(), Some("M"));
    assert_eq!(
        tree.nodes()
            .iter()
            .find(|node| node.path == "/unrelated.rs")
            .unwrap()
            .change_status,
        None
    );
}
