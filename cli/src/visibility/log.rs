use crate::{
    api::{
        self, ApiSession, HistoryEntryKind, HistoryEntrySummary, HistoryVisibilitySummary,
        VisibilityHistoryPage, api_url, http_client,
    },
    display::terminal_text,
    execution,
    login::session_from_cache_or_browser,
    repository_views::repository_views,
};
use anyhow::Result;
use scope_domain::views::{ViewId, Views};

pub(super) fn run(remote: Option<&str>, before: Option<&str>) -> Result<()> {
    let repo = crate::context::discover_optional()?;
    let target = crate::context::resolve_repository(repo.as_ref(), remote)?;
    let api_url = api_url()?;
    let client = http_client()?;
    let session = session_from_cache_or_browser(&client, &api_url)?;
    let api = ApiSession::new(&client, &api_url, &session.token);
    let views = repository_views(&api::get_repo(api, &target.owner, &target.repo)?.views)?;
    let page = api::visibility_history(api, &target.owner, &target.repo, before)?;
    execution::emit("visibility.log", &page, lines(&page, &views))
}

fn lines(page: &VisibilityHistoryPage, views: &Views) -> Vec<String> {
    let view = ViewId::from(page.view.clone());
    let measured = if &view == views.full() {
        views.anyone().unwrap_or(&view)
    } else {
        &view
    };
    let name = terminal_text(views.display_name(measured));
    let mut lines: Vec<_> = page
        .entries
        .iter()
        .map(|entry| entry_line(entry, &name))
        .collect();
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

fn entry_line(entry: &HistoryEntrySummary, view_name: &str) -> String {
    let mut parts = vec![terminal_text(&entry.source_id)];
    parts.extend(entry.occurred_at_unix.and_then(utc_minute));
    parts.extend(entry.author.as_deref().map(terminal_text));
    let message = entry.message.lines().next().unwrap_or_default();
    parts.push(terminal_text(message.trim()));
    if entry.kind != HistoryEntryKind::VisibilityChange {
        parts.push(summary_label(&entry.visibility_summary, view_name));
    }
    parts.join(" · ")
}

fn summary_label(summary: &HistoryVisibilitySummary, view_name: &str) -> String {
    [
        (summary.entered_count, "entered"),
        (summary.left_count, "left"),
    ]
    .into_iter()
    .filter(|(count, _)| *count > 0)
    .map(|(count, label)| format!("{count} {label} {view_name}"))
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

    fn entry(
        kind: HistoryEntryKind,
        occurred_at_unix: Option<i64>,
        author: Option<&str>,
        message: &str,
        (entered_count, left_count): (usize, usize),
    ) -> HistoryEntrySummary {
        HistoryEntrySummary {
            occurred_at_unix,
            source_id: "vc_000041".into(),
            kind,
            author: author.map(Into::into),
            message: message.into(),
            file_change_count: entered_count + left_count,
            visibility_summary: HistoryVisibilitySummary {
                entered_count,
                left_count,
            },
        }
    }

    fn page(entries: Vec<HistoryEntrySummary>, next_cursor: Option<&str>) -> VisibilityHistoryPage {
        page_for(scope_api_contract::ViewId::private(), entries, next_cursor)
    }

    fn page_for(
        view: scope_api_contract::ViewId,
        entries: Vec<HistoryEntrySummary>,
        next_cursor: Option<&str>,
    ) -> VisibilityHistoryPage {
        VisibilityHistoryPage {
            view,
            entries,
            next_cursor: next_cursor.map(Into::into),
        }
    }

    #[test]
    fn private_lines_show_time_author_and_push_counts() {
        let lines = lines(
            &page(
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
            ),
            &Views::builtin(),
        );
        assert_eq!(
            lines,
            [
                "vc_000041 · 2026-09-23 16:40 UTC · adamblumoff · Made 3 files public",
                "vc_000041 · 2026-09-23 16:40 UTC · adamblumoff · Merge pull request #407 · 2 entered Public, 1 left Public",
            ]
        );
    }

    #[test]
    fn public_lines_omit_withheld_time_and_author() {
        let lines = lines(
            &page(
                vec![entry(
                    HistoryEntryKind::Push,
                    None,
                    None,
                    "Tighten docs",
                    (0, 1),
                )],
                None,
            ),
            &Views::builtin(),
        );
        assert_eq!(lines, ["vc_000041 · Tighten docs · 1 left Public"]);
    }

    #[test]
    fn empty_page_reports_no_changes_and_keeps_the_cursor_hint() {
        assert_eq!(
            lines(&page(vec![], None), &Views::builtin()),
            ["No visibility changes."]
        );
        assert_eq!(
            lines(&page(vec![], Some("cursor-1")), &Views::builtin()),
            [
                "No visibility changes.",
                "More changes: repeat with --before cursor-1"
            ]
        );
    }

    #[test]
    fn custom_view_lines_use_the_repository_view_name() {
        let views = Views::new(vec![
            scope_domain::views::Views::builtin()
                .iter()
                .next()
                .unwrap()
                .clone(),
            scope_domain::views::Views::builtin()
                .iter()
                .nth(1)
                .unwrap()
                .clone(),
            scope_domain::views::ViewDefinition {
                id: scope_domain::views::ViewId::parse("agent").unwrap(),
                name: "Agent".into(),
                includes: scope_domain::views::ViewIncludes::Some(
                    [scope_domain::views::ViewId::public()].into(),
                ),
                readers: scope_domain::views::ViewReaders::Assigned,
            },
        ])
        .unwrap();
        let lines = lines(
            &page_for(
                scope_api_contract::ViewId::parse("agent").unwrap(),
                vec![entry(
                    HistoryEntryKind::ViewsChange,
                    None,
                    None,
                    "Updated views",
                    (2, 0),
                )],
                None,
            ),
            &views,
        );
        assert_eq!(lines, ["vc_000041 · Updated views · 2 entered Agent"]);
    }

    #[test]
    fn view_names_with_control_characters_are_neutralized() {
        let mut definitions = Vec::from(Views::builtin());
        definitions[0].name = "Pub\u{1b}[31mlic".into();
        let views = Views::new(definitions).unwrap();
        let lines = lines(
            &page(
                vec![entry(
                    HistoryEntryKind::Push,
                    None,
                    None,
                    "Tighten docs",
                    (0, 1),
                )],
                None,
            ),
            &views,
        );
        assert_eq!(lines, ["vc_000041 · Tighten docs · 1 left Pub [31mlic"]);
    }
}
