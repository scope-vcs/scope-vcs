use super::*;
use crate::error::DomainErrorKind;

fn range(start: u32, end: u32) -> RequestLineRange {
    RequestLineRange::new(start, end).unwrap()
}

fn numbered(lines: &[&str]) -> String {
    lines.iter().map(|line| format!("{line}\n")).collect()
}

#[test]
fn line_ranges_are_one_based_and_ordered() {
    assert_eq!(
        RequestLineRange::new(0, 1).unwrap_err().kind,
        DomainErrorKind::InvalidInput
    );
    assert_eq!(
        RequestLineRange::new(3, 2).unwrap_err().kind,
        DomainErrorKind::InvalidInput
    );
    assert_eq!(range(2, 2).end_line(), 2);
    assert!(serde_json::from_str::<RequestLineRange>(r#"{"start_line":3,"end_line":2}"#).is_err());
}

#[test]
fn line_anchors_require_a_path_but_paths_do_not_require_a_commit() {
    let lines = RequestDiscussionLines {
        side: RequestDiffSide::New,
        range: range(1, 2),
    };
    let anchor = |path: Option<&str>| RequestDiscussionAnchor {
        revision_id: "revision".to_string(),
        commit_oid: None,
        path: path.map(|path| ScopePath::parse(path).unwrap()),
        lines: Some(lines),
    };

    assert_eq!(
        anchor(None).validate().unwrap_err().kind,
        DomainErrorKind::InvalidInput
    );
    assert_eq!(anchor(Some("/src/lib.rs")).validate(), Ok(()));
}

#[test]
fn line_targets_must_be_text_on_the_side_and_contain_the_range() {
    let lines = RequestDiscussionLines {
        side: RequestDiffSide::Old,
        range: range(2, 3),
    };

    for target in [
        RequestLineTarget::Missing,
        RequestLineTarget::Binary,
        RequestLineTarget::Text("one\ntwo\n"),
    ] {
        assert_eq!(
            lines.ensure_target(target).unwrap_err().kind,
            DomainErrorKind::InvalidInput
        );
    }
    assert_eq!(
        lines.ensure_target(RequestLineTarget::Text("one\ntwo\nthree")),
        Ok(())
    );
}

#[test]
fn carry_forward_keeps_identical_text_and_shifts_past_insertions() {
    let original = numbered(&["a", "b", "c", "d", "e"]);
    assert_eq!(
        carry_forward_line_range(range(2, 3), &original, Some(&original)),
        RequestLinePosition::Mapped(range(2, 3))
    );

    let inserted_before = numbered(&["new", "new", "a", "b", "c", "d", "e"]);
    assert_eq!(
        carry_forward_line_range(range(2, 3), &original, Some(&inserted_before)),
        RequestLinePosition::Mapped(range(4, 5))
    );

    let edited_around = numbered(&["A", "x", "b", "c", "D", "e", "f"]);
    assert_eq!(
        carry_forward_line_range(range(2, 3), &original, Some(&edited_around)),
        RequestLinePosition::Mapped(range(3, 4))
    );

    let edited_after = numbered(&["a", "b", "c", "changed", "e"]);
    assert_eq!(
        carry_forward_line_range(range(2, 3), &original, Some(&edited_after)),
        RequestLinePosition::Mapped(range(2, 3))
    );
}

#[test]
fn carry_forward_marks_changed_or_missing_ranges_outdated() {
    let original = numbered(&["a", "b", "c", "d", "e"]);
    for newer in [
        numbered(&["a", "b", "changed", "d", "e"]),
        numbered(&["a", "b", "d", "e"]),
        numbered(&["a", "b", "inserted", "c", "d", "e"]),
        "a\nb\nc".to_string(),
    ] {
        assert_eq!(
            carry_forward_line_range(range(2, 3), &original, Some(&newer)),
            RequestLinePosition::Outdated,
            "{newer:?}"
        );
    }
    assert_eq!(
        carry_forward_line_range(range(2, 3), &original, None),
        RequestLinePosition::Outdated
    );
}

#[test]
fn carry_forward_gives_up_on_rewrites_too_large_to_prove() {
    let original = (0..30_000)
        .map(|line| format!("old {line}\n"))
        .collect::<String>();
    let rewritten = (0..30_000)
        .map(|line| format!("new {line}\n"))
        .collect::<String>();
    let kept_tail = format!("{rewritten}tail\n");
    let original_tail = format!("{original}tail\n");

    assert_eq!(
        carry_forward_line_range(range(2, 2), &original, Some(&rewritten)),
        RequestLinePosition::Outdated
    );
    assert_eq!(
        carry_forward_line_range(range(30_001, 30_001), &original_tail, Some(&kept_tail)),
        RequestLinePosition::Mapped(range(30_001, 30_001))
    );
}
