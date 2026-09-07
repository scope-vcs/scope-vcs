use super::text::terminal_text;
use scope_api_contract::{ReviewFileContentResponse, ReviewFileDiffResponse};

pub(super) fn file_diff_lines(diff: &ReviewFileDiffResponse) -> Vec<String> {
    let path = terminal_text(&diff.path);
    let mut lines = vec![format!("diff --git a/{path} b/{path}")];
    if diff.old_mode != diff.new_mode {
        if let Some(mode) = &diff.old_mode {
            lines.push(format!("old mode {}", terminal_text(mode)));
        }
        if let Some(mode) = &diff.new_mode {
            lines.push(format!("new mode {}", terminal_text(mode)));
        }
    }
    let old = text_content(&diff.old_content);
    let new = text_content(&diff.new_content);
    match (old, new) {
        (Some(old), Some(new)) => {
            let old_path = if diff.old_content.is_some() {
                format!("a/{path}")
            } else {
                "/dev/null".to_string()
            };
            let new_path = if diff.new_content.is_some() {
                format!("b/{path}")
            } else {
                "/dev/null".to_string()
            };
            if old != new {
                lines.push(format!("--- {old_path}"));
                lines.push(format!("+++ {new_path}"));
                lines.extend(text_hunk(old, new));
            }
        }
        _ => {
            for (side, content) in [("Before", &diff.old_content), ("After", &diff.new_content)] {
                if let Some(ReviewFileContentResponse::Binary { oid, size_bytes }) = content {
                    lines.push(format!(
                        "{side}: binary {} ({size_bytes} bytes)",
                        terminal_text(oid)
                    ));
                }
            }
            match (old, new) {
                (Some(text), None) if !text.is_empty() => {
                    lines.push(format!("--- a/{path} (text)"));
                    lines.extend(text_hunk(text, ""));
                }
                (None, Some(text)) if !text.is_empty() => {
                    lines.push(format!("+++ b/{path} (text)"));
                    lines.extend(text_hunk("", text));
                }
                _ => {}
            }
        }
    }
    lines
}

fn text_content(content: &Option<ReviewFileContentResponse>) -> Option<&str> {
    match content {
        None => Some(""),
        Some(ReviewFileContentResponse::Text { text }) => Some(text),
        Some(ReviewFileContentResponse::Binary { .. }) => None,
    }
}

// One hunk spanning the changed lines keeps rendering linear even for large files.
// File/revision selection and visibility always come from the API.
fn text_hunk(old: &str, new: &str) -> Vec<String> {
    let old = old.split_inclusive('\n').collect::<Vec<_>>();
    let new = new.split_inclusive('\n').collect::<Vec<_>>();
    let prefix = old
        .iter()
        .zip(&new)
        .take_while(|(left, right)| left == right)
        .count();
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(left, right)| left == right)
        .count();
    let start = prefix.saturating_sub(3);
    let old_changed_end = old.len() - suffix;
    let new_changed_end = new.len() - suffix;
    let context = suffix.min(3);
    let old_count = old_changed_end + context - start;
    let new_count = new_changed_end + context - start;
    let mut lines = vec![format!(
        "@@ -{},{} +{},{} @@",
        start + usize::from(old_count > 0),
        old_count,
        start + usize::from(new_count > 0),
        new_count
    )];
    append_lines(&mut lines, ' ', &old[start..prefix]);
    append_lines(&mut lines, '-', &old[prefix..old_changed_end]);
    append_lines(&mut lines, '+', &new[prefix..new_changed_end]);
    append_lines(
        &mut lines,
        ' ',
        &old[old_changed_end..old_changed_end + context],
    );
    lines
}

fn append_lines(output: &mut Vec<String>, prefix: char, lines: &[&str]) {
    for line in lines {
        output.push(format!(
            "{prefix}{}",
            terminal_text(line.strip_suffix('\n').unwrap_or(line))
        ));
        if !line.ends_with('\n') {
            output.push("\\ No newline at end of file".to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_text_conversions_render_both_sides() {
        let mut diff = ReviewFileDiffResponse {
            path: "converted.dat".into(),
            kind: scope_api_contract::FileChangeKind::Modified,
            old_mode: Some("100644".into()),
            new_mode: Some("100644".into()),
            old_content: Some(ReviewFileContentResponse::Binary {
                oid: "blob".into(),
                size_bytes: 3,
            }),
            new_content: Some(ReviewFileContentResponse::Text {
                text: "visible text\n".into(),
            }),
        };
        let added = file_diff_lines(&diff).join("\n");
        assert!(added.contains("Before: binary blob (3 bytes)"));
        assert!(added.contains("+visible text"));
        std::mem::swap(&mut diff.old_content, &mut diff.new_content);
        let removed = file_diff_lines(&diff).join("\n");
        assert!(removed.contains("After: binary blob (3 bytes)"));
        assert!(removed.contains("-visible text"));
    }

    #[test]
    fn text_diff_marks_additions_deletions_and_missing_newlines() {
        assert_eq!(text_hunk("", "new\n"), ["@@ -0,0 +1,1 @@", "+new"]);
        assert_eq!(text_hunk("old\n", ""), ["@@ -1,1 +0,0 @@", "-old"]);
        assert_eq!(
            text_hunk("same\nold", "same\nnew\n"),
            [
                "@@ -1,2 +1,2 @@",
                " same",
                "-old",
                "\\ No newline at end of file",
                "+new"
            ]
        );
    }

    #[test]
    fn hunk_limits_unchanged_context_and_preserves_control_safety() {
        assert_eq!(
            text_hunk(
                "1\n2\n3\n4\nold\n5\n6\n7\n8\n",
                "1\n2\n3\n4\nnew\u{1b}[31m\n5\n6\n7\n8\n"
            ),
            [
                "@@ -2,7 +2,7 @@",
                " 2",
                " 3",
                " 4",
                "-old",
                "+new [31m",
                " 5",
                " 6",
                " 7"
            ]
        );
    }
}
