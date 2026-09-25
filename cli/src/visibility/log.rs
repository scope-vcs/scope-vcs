use crate::{
    api::{
        self, ApiSession, HistoryEntryKind, HistoryEntrySummary, HistoryVisibilitySummary,
        VisibilityHistoryPage, api_url, http_client,
    },
    display::terminal_text,
    execution,
    login::session_from_cache_or_browser,
};
use anyhow::Result;

pub(super) fn run(remote: Option<&str>, before: Option<&str>) -> Result<()> {
    let repo = crate::context::discover_optional()?;
    let target = crate::context::resolve_repository(repo.as_ref(), remote)?;
    let api_url = api_url()?;
    let client = http_client()?;
    let session = session_from_cache_or_browser(&client, &api_url)?;
    let page = api::visibility_history(
        ApiSession::new(&client, &api_url, &session.token),
        &target.owner,
        &target.repo,
        before,
    )?;
    execution::emit("visibility.log", &page, lines(&page))
}

fn lines(page: &VisibilityHistoryPage) -> Vec<String> {
    let mut lines: Vec<_> = page.entries.iter().map(entry_line).collect();
    if lines.is_empty() {
        lines.push("No visibility changes.".into());
    }
    if let Some(cursor) = &page.next_cursor {
        lines.push(format!(
            "More changes: repeat with --before {}",
            terminal_text(cursor)
        ));
    }
    lines
}

fn entry_line(entry: &HistoryEntrySummary) -> String {
    let mut parts = vec![terminal_text(&entry.source_id)];
    parts.extend(entry.occurred_at_unix.and_then(utc_minute));
    parts.extend(entry.author.as_deref().map(terminal_text));
    let message = entry.message.lines().next().unwrap_or_default();
    parts.push(terminal_text(message.trim()));
    if entry.kind != HistoryEntryKind::VisibilityChange {
        parts.push(summary_label(&entry.visibility_summary));
    }
    parts.join(" · ")
}

fn summary_label(summary: &HistoryVisibilitySummary) -> String {
    [
        (summary.made_public_count, "made public"),
        (summary.made_private_count, "made private"),
    ]
    .into_iter()
    .filter(|(count, _)| *count > 0)
    .map(|(count, label)| format!("{count} {label}"))
    .collect::<Vec<_>>()
    .join(", ")
}

fn utc_minute(unix: i64) -> Option<String> {
    let time = time::OffsetDateTime::from_unix_timestamp(unix).ok()?;
    Some(format!(
        "{:04}-{:02}-{:02} {:02}:{:02} UTC",
        time.year(),
        u8::from(time.month()),
        time.day(),
        time.hour(),
        time.minute()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::HistoryAudience;

    fn entry(
        kind: HistoryEntryKind,
        occurred_at_unix: Option<i64>,
        author: Option<&str>,
        message: &str,
        (made_public_count, made_private_count): (usize, usize),
    ) -> HistoryEntrySummary {
        HistoryEntrySummary {
            occurred_at_unix,
            source_id: "vc_000041".into(),
            kind,
            author: author.map(Into::into),
            message: message.into(),
            file_change_count: made_public_count + made_private_count,
            visibility_summary: HistoryVisibilitySummary {
                made_public_count,
                made_private_count,
            },
        }
    }

    fn page(entries: Vec<HistoryEntrySummary>, next_cursor: Option<&str>) -> VisibilityHistoryPage {
        VisibilityHistoryPage {
            audience: HistoryAudience::Private,
            entries,
            next_cursor: next_cursor.map(Into::into),
        }
    }

    #[test]
    fn private_lines_show_time_author_and_push_counts() {
        let lines = lines(&page(
            vec![
                entry(
                    HistoryEntryKind::VisibilityChange,
                    Some(1_790_181_600),
                    Some("adamblumoff"),
                    "Made 3 files public",
                    (3, 0),
                ),
                entry(
                    HistoryEntryKind::MergedRequest,
                    Some(1_790_181_600),
                    Some("adamblumoff"),
                    "Merge pull request #407\n\nDetails",
                    (2, 1),
                ),
            ],
            None,
        ));
        assert_eq!(
            lines,
            [
                "vc_000041 · 2026-09-23 16:40 UTC · adamblumoff · Made 3 files public",
                "vc_000041 · 2026-09-23 16:40 UTC · adamblumoff · Merge pull request #407 · 2 made public, 1 made private",
            ]
        );
    }

    #[test]
    fn public_lines_omit_withheld_time_and_author() {
        let lines = lines(&page(
            vec![entry(
                HistoryEntryKind::Push,
                None,
                None,
                "Tighten docs",
                (0, 1),
            )],
            None,
        ));
        assert_eq!(lines, ["vc_000041 · Tighten docs · 1 made private"]);
    }

    #[test]
    fn empty_page_reports_no_changes_and_keeps_the_cursor_hint() {
        assert_eq!(lines(&page(vec![], None)), ["No visibility changes."]);
        assert_eq!(
            lines(&page(vec![], Some("cursor-1"))),
            [
                "No visibility changes.",
                "More changes: repeat with --before cursor-1"
            ]
        );
    }
}
