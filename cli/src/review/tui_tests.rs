use super::{footer_hints, review_body_heights, row_line};
use crate::review::state::{ChangeListKind, ReviewMode, ReviewRow};
use scope_domain::repo_visibility::ReviewVisibility;
use unicode_width::UnicodeWidthStr;

#[test]
fn body_layout_keeps_a_file_row_visible_when_read_only_summaries_overflow() {
    assert_eq!(review_body_heights(3, 10, 5), (2, 1));
    assert_eq!(review_body_heights(1, 10, 5), (0, 1));
}

#[test]
fn body_layout_uses_available_space_when_summaries_fit() {
    assert_eq!(review_body_heights(5, 2, 5), (2, 3));
    assert_eq!(review_body_heights(5, 10, 0), (5, 0));
}

#[test]
fn tree_rows_use_web_visibility_icons_and_stay_within_terminal_width() {
    let row = |visibility| ReviewRow::TreeNode {
        depth: 1,
        name: "a-very-long-file-name-that-needs-to-be-truncated.rs".to_string(),
        path: "/a-very-long-file-name-that-needs-to-be-truncated.rs".to_string(),
        kind: crate::review::tree::ReviewNodeKind::File,
        expanded: false,
        visibility,
        rule: "inherited /some/very/long/folder/**".to_string(),
        reserved: false,
        change_status: Some("A".to_string()),
    };

    let public_line = row_line(&row(ReviewVisibility::Public), false, 80).to_string();
    let private_line = row_line(&row(ReviewVisibility::Private), false, 80).to_string();
    assert!(public_line.contains("🌐 public"), "{public_line}");
    assert!(private_line.contains("🔒 private"), "{private_line}");
    assert!(!public_line.ends_with("  A"), "{public_line}");
    assert_eq!(UnicodeWidthStr::width(public_line.as_str()), 80);
    assert_eq!(UnicodeWidthStr::width(private_line.as_str()), 80);
}

#[test]
fn change_section_rows_are_compact_and_descriptive() {
    let row = ReviewRow::ChangeSection {
        kind: ChangeListKind::Deleted,
        count: 87,
        expanded: false,
    };
    let line = row_line(&row, false, 80).to_string();

    assert!(line.starts_with("[>] Deleted files (87)"), "{line}");
    assert_eq!(UnicodeWidthStr::width(line.as_str()), 80);
}

#[test]
fn change_rows_escape_control_characters_without_splitting_literal_arrows() {
    let row = ReviewRow::ChangePath {
        kind: ChangeListKind::Added,
        path: " old -> new\t\n\u{1b}[31m.rs".to_string(),
    };
    let line = row_line(&row, false, 100).to_string();
    assert!(line.contains("old -> new\\t\\n\\u{1b}[31m.rs"), "{line:?}");
    assert!(!line.chars().any(char::is_control), "{line:?}");
    assert_eq!(UnicodeWidthStr::width(line.as_str()), 100);
}

#[test]
fn narrow_footer_keeps_required_push_actions_visible() {
    let hints = footer_hints(ReviewMode::Push, 40);
    let text = hints.join(" ");

    assert!(hints.len() > 1, "{hints:?}");
    assert!(
        hints
            .iter()
            .all(|line| UnicodeWidthStr::width(line.as_str()) <= 40)
    );
    for control in [
        "↑↓←→ move",
        "Space toggle",
        "S save",
        "P push",
        "Q cancel",
        "/ filter",
        "? help",
    ] {
        assert!(text.contains(control), "missing {control}: {hints:?}");
    }
}
