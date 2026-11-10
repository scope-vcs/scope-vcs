use super::{diff_side_label, line_span};
use crate::display::terminal_text;
use scope_api_contract::{
    RequestDiscussionAnchor, RequestDiscussionAuthorResponse, RequestDiscussionStatus,
    RequestDiscussionSummaryResponse,
};

pub(in crate::request) fn list_lines(
    discussions: &[RequestDiscussionSummaryResponse],
) -> Vec<String> {
    if discussions.is_empty() {
        return vec!["No discussions.".to_string()];
    }
    discussions.iter().map(list_line).collect()
}

fn list_line(discussion: &RequestDiscussionSummaryResponse) -> String {
    let replies = match discussion.reply_count {
        1 => "1 reply".to_string(),
        count => format!("{count} replies"),
    };
    let first_line = discussion.body_markdown.lines().next().unwrap_or_default();
    format!(
        "{} · {} · {} · {} · {replies} · {}",
        terminal_text(&discussion.id),
        status_label(discussion.status),
        author_label(discussion.author.as_ref()),
        anchor_label(discussion.anchor.as_ref()),
        terminal_text(first_line),
    )
}

fn status_label(status: RequestDiscussionStatus) -> &'static str {
    match status {
        RequestDiscussionStatus::Open => "open",
        RequestDiscussionStatus::Resolved => "resolved",
    }
}

fn author_label(author: Option<&RequestDiscussionAuthorResponse>) -> String {
    match author {
        Some(RequestDiscussionAuthorResponse::User { handle, .. }) => {
            format!("@{}", terminal_text(handle))
        }
        Some(RequestDiscussionAuthorResponse::GitHub { account, .. }) => {
            format!("{} via GitHub", terminal_text(&account.login))
        }
        None => "deleted user".to_string(),
    }
}

fn anchor_label(anchor: Option<&RequestDiscussionAnchor>) -> String {
    let Some(anchor) = anchor else {
        return "request".to_string();
    };
    let Some(path) = anchor.path.as_deref() else {
        return format!("revision {}", terminal_text(&anchor.revision_id));
    };
    match anchor.lines.as_ref() {
        Some(lines) => format!(
            "{}:{} ({})",
            terminal_text(path),
            line_span(lines.start_line, lines.end_line),
            diff_side_label(lines.side)
        ),
        None => terminal_text(path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn discussion(overrides: serde_json::Value) -> RequestDiscussionSummaryResponse {
        let mut value = json!({
            "id": "dsc_one", "request_id": "req_one", "client_discussion_id": "client_one",
            "opened_position": 1, "last_activity_position": 1,
            "author": {"kind": "user", "id": "usr_one", "handle": "river"},
            "body_markdown": "First line\nSecond line", "anchor": null, "status": "Open",
            "reply_count": 2, "read_through_position": 0, "unread_count": 0,
            "latest_replies": [], "created_at_unix": 1, "resolved_at_unix": null,
            "resolved_by": null
        });
        value
            .as_object_mut()
            .unwrap()
            .extend(overrides.as_object().unwrap().clone());
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn lines_carry_id_status_author_anchor_replies_and_the_first_body_line() {
        let anchored = discussion(json!({
            "id": "dsc_two", "status": "Resolved", "reply_count": 1,
            "author": {"kind": "github", "account": {"github_user_id": 7, "login": "review-bot", "account_type": "bot"}, "source_url": "https://example.test/c"},
            "anchor": {
                "revision_id": "rev_1", "revision_position": 1, "commit_oid": null,
                "path": "src/lib.rs", "latest": null,
                "lines": {"side": "new", "start_line": 7, "end_line": 9}
            }
        }));

        assert_eq!(
            list_lines(&[discussion(json!({})), anchored]),
            [
                "dsc_one · open · @river · request · 2 replies · First line",
                "dsc_two · resolved · review-bot via GitHub · src/lib.rs:7-9 (new) · 1 reply · First line",
            ]
        );
    }

    #[test]
    fn an_empty_list_says_so() {
        assert_eq!(list_lines(&[]), ["No discussions."]);
    }
}
