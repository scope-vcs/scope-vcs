use super::*;
use crate::git_repo::GitChangedPath;
use crate::repo_config::default_scope_repo_config;
use scope_domain::{
    dependency_analysis::{
        AnalyzerOutput, DEPENDENCY_ANALYZER_VERSION, DependencyEdge, DependencyEdgeKind,
        DependencyGap, StoredDependencyAnalysis,
    },
    repo_config::{
        ConfigVisibility, HistoryRewriteAction, HistoryRewriteRequest, RepoConfigVisibilityRule,
    },
};

fn state_with_mode(mode: ReviewMode) -> ReviewState {
    let tree = ReviewTree::from_paths(&["src/lib.rs".to_string(), "README.md".to_string()], &[]);
    ReviewState::new(tree, default_scope_repo_config(), mode)
}

fn state() -> ReviewState {
    state_with_mode(ReviewMode::Standalone)
}

#[test]
fn rename_and_copy_changes_show_destination_and_only_rename_deletes_source() {
    let changes = [
        GitChangedPath {
            status: "R100".to_string(),
            path: "dest -> literal\n.rs".to_string(),
            previous_path: Some("old\t.rs".to_string()),
        },
        GitChangedPath {
            status: "C100".to_string(),
            path: "copy.rs".to_string(),
            previous_path: Some("original.rs".to_string()),
        },
    ];
    let (added, deleted) = split_change_paths(&changes);
    assert_eq!(added, vec!["copy.rs", "dest -> literal\n.rs"]);
    assert_eq!(deleted, vec!["old\t.rs"]);
}

fn tree_path(row: &ReviewRow) -> Option<&str> {
    match row {
        ReviewRow::TreeNode { path, .. } => Some(path),
        ReviewRow::DependencySummary(_)
        | ReviewRow::DependencyFinding { .. }
        | ReviewRow::DependencyGap(_)
        | ReviewRow::DependencyCoverage { .. }
        | ReviewRow::ChangeSection { .. }
        | ReviewRow::ChangePath { .. } => None,
    }
}

fn state_with_changes() -> ReviewState {
    let changed_paths = vec![
        GitChangedPath {
            status: "D".to_string(),
            path: "old.txt".to_string(),
            previous_path: None,
        },
        GitChangedPath {
            status: "A".to_string(),
            path: "src/new.rs".to_string(),
            previous_path: None,
        },
    ];
    let tree = ReviewTree::from_paths(
        &["src/new.rs".to_string(), "README.md".to_string()],
        &changed_paths,
    );
    ReviewState::build(
        tree,
        default_scope_repo_config(),
        ReviewMode::Standalone,
        &changed_paths,
        DependencyReview::hidden(),
    )
}

#[test]
fn right_arrow_expands_folder_and_moves_to_first_child_when_already_expanded() {
    let mut state = state();
    state.handle_input(ReviewInput::Down);
    assert_eq!(
        tree_path(&state.visible_rows(0, usize::MAX)[state.cursor()]),
        Some("/src")
    );

    state.handle_input(ReviewInput::Right);
    assert!(
        state
            .visible_rows(0, usize::MAX)
            .iter()
            .any(|row| tree_path(row) == Some("/src/lib.rs"))
    );

    state.handle_input(ReviewInput::Right);
    assert_eq!(
        tree_path(&state.visible_rows(0, usize::MAX)[state.cursor()]),
        Some("/src/lib.rs")
    );
}

#[test]
fn quit_respects_dirty_state_and_push_mode() {
    let mut state = state();
    state.handle_input(ReviewInput::Toggle);
    assert_eq!(
        state.handle_input(ReviewInput::Quit),
        ReviewStateAction::None
    );
    assert!(state.message().contains("Unsaved changes"));
    assert_eq!(
        state.handle_input(ReviewInput::Escape),
        ReviewStateAction::Cancel
    );
    assert_eq!(
        state_with_mode(ReviewMode::Push).handle_input(ReviewInput::Quit),
        ReviewStateAction::Cancel,
    );
}

#[test]
fn escape_clears_closed_filter_before_canceling() {
    let mut state = state();

    state.handle_input(ReviewInput::Filter);
    state.handle_input(ReviewInput::Char('s'));
    assert_eq!(state.filter(), "s");
    assert!(state.editing_filter());

    assert_eq!(
        state.handle_input(ReviewInput::Escape),
        ReviewStateAction::None
    );
    assert_eq!(state.filter(), "s");
    assert!(!state.editing_filter());

    assert_eq!(
        state.handle_input(ReviewInput::Escape),
        ReviewStateAction::None
    );
    assert_eq!(state.filter(), "");
    assert!(state.message().contains("Filter cleared"));

    assert_eq!(
        state.handle_input(ReviewInput::Escape),
        ReviewStateAction::Cancel
    );
}

#[test]
fn initial_message_surfaces_read_only_history_rewrites() {
    let tree = ReviewTree::from_paths(&["README.md".to_string()], &[]);
    let mut config = default_scope_repo_config();
    config.history.rewrites.push(HistoryRewriteRequest {
        path: "/secret.txt".into(),
        action: HistoryRewriteAction::RedactPublicHistory,
    });

    let state = ReviewState::new(tree, config, ReviewMode::Push);

    assert!(state.message().contains("history rewrite"));
    assert_eq!(state.history_rewrite_count(), 1);
    assert_eq!(
        state.history_rewrite_summaries(),
        vec!["History rewrite: /secret.txt -> redact public history".to_string()]
    );
}

#[test]
fn added_and_deleted_sections_start_collapsed() {
    let state = state_with_changes();
    let rows = state.visible_rows(0, usize::MAX);

    assert!(matches!(
        rows[0],
        ReviewRow::ChangeSection {
            kind: ChangeListKind::Added,
            count: 1,
            expanded: false,
        }
    ));
    assert!(matches!(
        rows[1],
        ReviewRow::ChangeSection {
            kind: ChangeListKind::Deleted,
            count: 1,
            expanded: false,
        }
    ));
    assert!(
        !rows
            .iter()
            .any(|row| matches!(row, ReviewRow::ChangePath { .. }))
    );
    assert!(matches!(
        rows[state.cursor()],
        ReviewRow::TreeNode { ref path, .. } if path == "/"
    ));
}

#[test]
fn change_sections_expand_and_paths_remain_informational() {
    let mut state = state_with_changes();

    state.handle_input(ReviewInput::Up);
    state.handle_input(ReviewInput::Up);
    state.handle_input(ReviewInput::Right);
    assert!(matches!(
        state.visible_rows(0, usize::MAX)[1],
        ReviewRow::ChangePath {
            kind: ChangeListKind::Added,
            ref path,
        } if path == "src/new.rs"
    ));

    state.handle_input(ReviewInput::Right);
    assert_eq!(state.cursor(), 1);
    state.handle_input(ReviewInput::Toggle);
    assert!(!state.is_dirty());
    assert!(state.message().contains("informational"));

    state.handle_input(ReviewInput::Left);
    assert_eq!(state.cursor(), 0);
    state.handle_input(ReviewInput::Left);
    assert!(
        !state
            .visible_rows(0, usize::MAX)
            .iter()
            .any(|row| matches!(row, ReviewRow::ChangePath { .. }))
    );
}

#[test]
fn filtering_surfaces_matching_change_paths_without_persisting_expansion() {
    let mut state = state_with_changes();

    state.handle_input(ReviewInput::Filter);
    for value in "old".chars() {
        state.handle_input(ReviewInput::Char(value));
    }
    let rows = state.visible_rows(0, usize::MAX);
    assert!(matches!(
        rows[0],
        ReviewRow::ChangeSection {
            kind: ChangeListKind::Deleted,
            expanded: true,
            ..
        }
    ));
    assert!(matches!(
        rows[1],
        ReviewRow::ChangePath {
            kind: ChangeListKind::Deleted,
            ref path,
        } if path == "old.txt"
    ));

    state.handle_input(ReviewInput::Escape);
    state.handle_input(ReviewInput::Escape);
    let rows = state.visible_rows(0, usize::MAX);
    assert!(matches!(
        rows[1],
        ReviewRow::ChangeSection {
            kind: ChangeListKind::Deleted,
            expanded: false,
            ..
        }
    ));
}

fn assert_cached_visibility_matches_domain(state: &ReviewState) {
    use scope_domain::repo_visibility::{VisibilityNodeKind, VisibilityTarget, target_visibility};
    for node in state.tree.nodes() {
        let expected = target_visibility(
            state.config(),
            &VisibilityTarget {
                name: &node.name,
                path: &node.path,
                kind: match node.kind {
                    ReviewNodeKind::Root => VisibilityNodeKind::Root,
                    ReviewNodeKind::Directory => VisibilityNodeKind::Directory,
                    ReviewNodeKind::File => VisibilityNodeKind::File,
                },
                reserved: node.reserved,
                file_paths_under: state.tree.file_paths_under(node.id),
            },
        );
        assert_eq!(state.visibilities[node.id], expected, "{}", node.path);
    }
}

#[test]
fn cached_summaries_follow_root_directory_file_and_reserved_toggles() {
    let paths = [
        "src/public.rs",
        "src/private.rs",
        "src/**",
        ".scope/RULES.md",
        ".scope/runs/check.yml",
    ]
    .map(str::to_string);
    let mut state = ReviewState::new(
        ReviewTree::from_paths(&paths, &[]),
        default_scope_repo_config(),
        ReviewMode::Standalone,
    );
    state.handle_input(ReviewInput::Filter);
    state.handle_input(ReviewInput::Char('/'));
    state.handle_input(ReviewInput::Escape);
    assert_cached_visibility_matches_domain(&state);
    for path in [
        "/",
        "/src",
        "/src/public.rs",
        "/.scope/RULES.md",
        "/src/**",
        "/",
    ] {
        let id = state
            .tree
            .nodes()
            .iter()
            .find(|node| node.path == path)
            .unwrap()
            .id;
        state.move_cursor_to_item(ReviewItem::TreeNode(id));
        state.handle_input(ReviewInput::Toggle);
        assert_cached_visibility_matches_domain(&state);
    }
    let saved = state.config().clone();
    state.mark_saved();
    assert!(!state.is_dirty());
    assert_eq!(state.config(), &saved);
    assert_cached_visibility_matches_domain(&state);
}

#[test]
fn viewport_rows_match_filtered_and_expanded_navigation() {
    let paths = (0..150)
        .map(|index| format!("src/file_{index:03}.rs"))
        .collect::<Vec<_>>();
    let mut state = ReviewState::new(
        ReviewTree::from_paths(&paths, &[]),
        default_scope_repo_config(),
        ReviewMode::Standalone,
    );
    for filtered in [false, true] {
        if filtered {
            state.handle_input(ReviewInput::Filter);
            state.handle_input(ReviewInput::Char('s'));
            state.handle_input(ReviewInput::Escape);
        } else {
            state.handle_input(ReviewInput::Down);
            state.handle_input(ReviewInput::Right);
        }
        let summaries = state.visibilities.as_ptr();
        for _ in 0..120 {
            state.handle_input(ReviewInput::Down);
            state.adjust_scroll(12);
            let all = state.visible_rows(0, usize::MAX);
            assert_eq!(all.len(), state.visible_row_count());
            let expected = all
                .iter()
                .skip(state.scroll())
                .take(12)
                .cloned()
                .collect::<Vec<_>>();
            assert_eq!(state.visible_rows(state.scroll(), 12), expected);
            assert_eq!(state.visibilities.as_ptr(), summaries);
        }
        assert!(state.visible_rows(state.visible_row_count(), 12).is_empty());
        assert!(state.visible_rows(0, 0).is_empty());
    }
}

#[test]
fn empty_tree_summary_uses_config_default() {
    let mut state = ReviewState::new(
        ReviewTree::from_paths(&[], &[]),
        default_scope_repo_config(),
        ReviewMode::Standalone,
    );
    assert_cached_visibility_matches_domain(&state);
    state.handle_input(ReviewInput::Toggle);
    assert_cached_visibility_matches_domain(&state);
}

fn dependency_config() -> RepoConfig {
    let mut config = RepoConfig::with_default_visibility(ConfigVisibility::Public);
    config.visibility.rules.push(RepoConfigVisibilityRule {
        path: "/private/**".into(),
        visibility: ConfigVisibility::Private,
    });
    config
}

fn dependency_analysis(
    commit_oid: &str,
    edges: Vec<DependencyEdge>,
    gaps: Vec<DependencyGap>,
    analyzed_files: Vec<&str>,
    unsupported_files: Vec<&str>,
) -> StoredDependencyAnalysis {
    StoredDependencyAnalysis::from_output(
        commit_oid,
        AnalyzerOutput {
            analyzer_version: DEPENDENCY_ANALYZER_VERSION.into(),
            analyzed_files: analyzed_files.into_iter().map(str::to_string).collect(),
            unsupported_files: unsupported_files.into_iter().map(str::to_string).collect(),
            edges,
            gaps,
        },
    )
    .unwrap()
}

fn dependency_state() -> ReviewState {
    let paths = ["public/a.ts", "public/b.ts", "private/secret.ts"].map(str::to_string);
    ReviewState::new_push(
        ReviewTree::from_paths(&paths, &[]),
        dependency_config(),
        &[],
        "reviewed".into(),
    )
}

#[test]
fn dependency_results_preserve_pending_review_navigation_and_unsaved_config() {
    let mut state = dependency_state();
    state.handle_input(ReviewInput::Down);
    state.handle_input(ReviewInput::Toggle);
    state.handle_input(ReviewInput::Filter);
    for character in "public".chars() {
        state.handle_input(ReviewInput::Char(character));
    }
    state.handle_input(ReviewInput::Escape);
    let cursor = state.cursor();
    let config = state.config().clone();

    state.complete_dependency_analysis(Ok(dependency_analysis(
        "reviewed",
        vec![],
        vec![],
        vec!["public/a.ts"],
        vec![],
    )));

    assert_eq!(state.cursor(), cursor);
    assert_eq!(state.filter(), "public");
    assert_eq!(state.config(), &config);
    assert!(state.is_dirty());
}

#[test]
fn findings_recompute_from_cached_analysis_and_keep_selected_tree_path() {
    let mut state = dependency_state();
    state.complete_dependency_analysis(Ok(dependency_analysis(
        "reviewed",
        vec![DependencyEdge {
            source_path: "public/a.ts".into(),
            target_path: "private/secret.ts".into(),
            kind: DependencyEdgeKind::Import,
        }],
        vec![],
        vec!["public/a.ts", "private/secret.ts"],
        vec![],
    )));
    state.handle_input(ReviewInput::Dependencies);
    state.handle_input(ReviewInput::Toggle);
    assert!(matches!(
        state.visible_rows(0, usize::MAX)[1],
        ReviewRow::DependencyFinding { .. }
    ));

    state.handle_input(ReviewInput::Down);
    state.handle_input(ReviewInput::Right);
    state.handle_input(ReviewInput::Toggle);
    assert_eq!(
        tree_path(&state.visible_rows(0, usize::MAX)[state.cursor()]),
        Some("/private/secret.ts")
    );

    state.handle_input(ReviewInput::Toggle);
    assert_eq!(
        tree_path(&state.visible_rows(0, usize::MAX)[state.cursor()]),
        Some("/private/secret.ts")
    );
    assert!(
        state
            .visible_rows(0, usize::MAX)
            .iter()
            .all(|row| !matches!(row, ReviewRow::DependencyFinding { .. }))
    );
    assert!(matches!(
        &state.visible_rows(0, 1)[0],
        ReviewRow::DependencySummary(summary)
            if summary.label == "No public → private imports found"
    ));
}

#[test]
fn incomplete_result_keeps_known_findings_and_coverage_gaps() {
    let mut state = dependency_state();
    state.complete_dependency_analysis(Ok(dependency_analysis(
        "reviewed",
        vec![DependencyEdge {
            source_path: "public/a.ts".into(),
            target_path: "private/secret.ts".into(),
            kind: DependencyEdgeKind::Import,
        }],
        vec![DependencyGap {
            path: "public/b.ts".into(),
            reason: "unresolved alias".into(),
        }],
        vec!["public/a.ts", "public/b.ts", "private/secret.ts"],
        vec!["src/main.rs"],
    )));
    state.handle_input(ReviewInput::Dependencies);
    state.handle_input(ReviewInput::Toggle);
    let rows = state.visible_rows(0, usize::MAX);

    assert!(matches!(
        &rows[0],
        ReviewRow::DependencySummary(summary)
            if summary.label == "1 public file imports private files"
                && summary.meta.as_deref() == Some("Check incomplete")
    ));
    assert!(
        rows.iter()
            .any(|row| matches!(row, ReviewRow::DependencyFinding { .. }))
    );
    assert!(rows.iter().any(|row| matches!(
        row,
        ReviewRow::DependencyGap(gap) if gap.path == "public/b.ts"
    )));
    assert!(rows.iter().any(|row| matches!(
        row,
        ReviewRow::DependencyCoverage {
            analyzed_file_count: 3,
            unsupported_file_count: 1,
        }
    )));
}

#[test]
fn stale_unavailable_and_unsupported_results_are_explicit_and_advisory() {
    let mut stale = dependency_state();
    stale.complete_dependency_analysis(Ok(dependency_analysis(
        "another-commit",
        vec![],
        vec![],
        vec!["public/a.ts"],
        vec![],
    )));
    assert!(matches!(
        &stale.visible_rows(0, 1)[0],
        ReviewRow::DependencySummary(summary)
            if summary.label == "Dependency check unavailable"
    ));
    assert_eq!(
        stale.handle_input(ReviewInput::ContinuePush),
        ReviewStateAction::ContinuePush
    );

    let mut unavailable = dependency_state();
    unavailable.complete_dependency_analysis(Err("analyzer failed".into()));
    assert!(matches!(
        &unavailable.visible_rows(0, 1)[0],
        ReviewRow::DependencySummary(summary)
            if summary.label == "Dependency check unavailable"
    ));

    let mut unsupported = dependency_state();
    unsupported.complete_dependency_analysis(Ok(dependency_analysis(
        "reviewed",
        vec![],
        vec![],
        vec![],
        vec!["src/main.rs"],
    )));
    assert!(matches!(
        &unsupported.visible_rows(0, 1)[0],
        ReviewRow::DependencySummary(summary)
            if summary.label.contains("does not support")
    ));

    let mut unsupported_with_gap = dependency_state();
    unsupported_with_gap.complete_dependency_analysis(Ok(dependency_analysis(
        "reviewed",
        vec![],
        vec![DependencyGap {
            path: ".".into(),
            reason: "analyzer could not read the project configuration".into(),
        }],
        vec![],
        vec!["src/main.rs"],
    )));
    assert!(matches!(
        &unsupported_with_gap.visible_rows(0, 1)[0],
        ReviewRow::DependencySummary(summary)
            if summary.label == "Dependency check incomplete"
    ));
}
