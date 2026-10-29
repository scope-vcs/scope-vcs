use crate::display::terminal_text;
use scope_api_contract::{CommitFileResponse, FileLineChangesResponse, RequestDiffStatResponse};

pub(super) fn queue_label(stat: &RequestDiffStatResponse) -> String {
    if stat.additions == 0 && stat.deletions == 0 && stat.changed_files > 0 {
        return files_label(stat.changed_files);
    }
    format!(
        "+{} -{}",
        abbreviated(stat.additions),
        abbreviated(stat.deletions)
    )
}

pub(super) fn detail_line(stat: &RequestDiffStatResponse) -> String {
    format!(
        "  changes: +{} -{} across {}",
        stat.additions,
        stat.deletions,
        files_label(stat.changed_files)
    )
}

fn files_label(count: u64) -> String {
    if count == 1 {
        "1 file".to_string()
    } else {
        format!("{count} files")
    }
}

fn abbreviated(count: u64) -> String {
    let (unit, suffix) = match count {
        0..1_000 => return count.to_string(),
        1_000..1_000_000 => (1_000, 'k'),
        _ => (1_000_000, 'm'),
    };
    let tenths = count / (unit / 10);
    match tenths % 10 {
        0 => format!("{}{suffix}", tenths / 10),
        fraction => format!("{}.{fraction}{suffix}", tenths / 10),
    }
}

pub(super) struct DiffStat {
    additions: u64,
    deletions: u64,
    changed_files: usize,
    path_width: usize,
}

impl DiffStat {
    pub(super) fn new<'a>(files: impl IntoIterator<Item = &'a CommitFileResponse>) -> Self {
        let mut stat = Self {
            additions: 0,
            deletions: 0,
            changed_files: 0,
            path_width: 0,
        };
        for file in files {
            stat.changed_files += 1;
            stat.path_width = stat
                .path_width
                .max(terminal_text(&file.path).chars().count());
            if let FileLineChangesResponse::Text {
                additions,
                deletions,
            } = file.lines
            {
                stat.additions += additions;
                stat.deletions += deletions;
            }
        }
        stat
    }

    pub(super) fn row(&self, file: &CommitFileResponse) -> String {
        let counts = match file.lines {
            FileLineChangesResponse::Text {
                additions,
                deletions,
            } => format!(
                "{:>addition_width$} {:>deletion_width$}",
                format!("+{additions}"),
                format!("-{deletions}"),
                addition_width = self.additions.to_string().len() + 1,
                deletion_width = self.deletions.to_string().len() + 1,
            ),
            FileLineChangesResponse::Binary => "binary".to_string(),
        };
        format!(
            " {:<width$} | {counts}",
            terminal_text(&file.path),
            width = self.path_width
        )
    }

    pub(super) fn summary(&self) -> Option<String> {
        (self.changed_files > 0).then(|| {
            format!(
                " {} changed, +{} -{}",
                files_label(self.changed_files as u64),
                self.additions,
                self.deletions
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stat(additions: u64, deletions: u64, changed_files: u64) -> RequestDiffStatResponse {
        RequestDiffStatResponse {
            additions,
            deletions,
            changed_files,
        }
    }

    fn file(path: &str, lines: serde_json::Value) -> CommitFileResponse {
        serde_json::from_value(serde_json::json!({
            "path": path, "kind": "Modified", "old_mode": "100644", "new_mode": "100644",
            "old_oid": null, "new_oid": null, "label": "public", "lines": lines
        }))
        .unwrap()
    }

    #[test]
    fn queue_label_abbreviates_large_counts_and_names_binary_only_changes() {
        for (counts, expected) in [
            (stat(128, 41, 6), "+128 -41"),
            (stat(999, 1_000, 2), "+999 -1k"),
            (stat(2_468, 12_345, 9), "+2.4k -12.3k"),
            (stat(1_999_999, 0, 1), "+1.9m -0"),
            (stat(0, 0, 3), "3 files"),
            (stat(0, 0, 1), "1 file"),
            (stat(0, 0, 0), "+0 -0"),
        ] {
            assert_eq!(queue_label(&counts), expected);
        }
    }

    #[test]
    fn detail_line_keeps_exact_counts() {
        assert_eq!(
            detail_line(&stat(12_345, 41, 6)),
            "  changes: +12345 -41 across 6 files"
        );
        assert_eq!(
            detail_line(&stat(1, 0, 1)),
            "  changes: +1 -0 across 1 file"
        );
    }

    #[test]
    fn stat_aligns_paths_and_counts_and_totals_text_files() {
        let files = [
            file(
                "api/src/http/request_review/comparison.rs",
                serde_json::json!({"kind": "text", "additions": 22, "deletions": 4}),
            ),
            file(
                "web/src/features/requests/request-workspace-list.tsx",
                serde_json::json!({"kind": "text", "additions": 19, "deletions": 6}),
            ),
            file(
                "web/public/logo\u{1b}.png",
                serde_json::json!({"kind": "binary"}),
            ),
        ];
        let stat = DiffStat::new(&files);
        let mut lines = files.iter().map(|file| stat.row(file)).collect::<Vec<_>>();
        lines.extend(stat.summary());
        assert_eq!(
            lines,
            [
                " api/src/http/request_review/comparison.rs            | +22  -4",
                " web/src/features/requests/request-workspace-list.tsx | +19  -6",
                " web/public/logo .png                                 | binary",
                " 3 files changed, +41 -10",
            ]
        );
    }
}
