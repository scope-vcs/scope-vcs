//! The request attention queue: which requests need the viewer, and the
//! commands that move one request within it. Groups and labels mirror the web
//! sidebar (`web/src/features/requests/request-workspace-model.ts`).

use super::*;
use crate::api::{
    RepositoryActor, RequestAttentionActionRequest, RequestAttentionReason,
    RequestQueueItemResponse, RequestQueueSection, apply_request_attention, request_queue_page,
};
use crate::display::terminal_text;
use crate::error::CliError;
use args::SnoozeFor;
use chrono::{DateTime, Datelike, Days, Local, NaiveTime, TimeZone};
use serde::Serialize;

pub(super) const QUEUE_SECTION_LIMIT: u32 = 30;

const DEFAULT_SECTIONS: [RequestQueueSection; 2] =
    [RequestQueueSection::Active, RequestQueueSection::Unclaimed];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum QueueGroup {
    NeedsYou,
    Waiting,
    Unclaimed,
    SetAside,
    Done,
}

const GROUP_ORDER: [QueueGroup; 5] = [
    QueueGroup::NeedsYou,
    QueueGroup::Waiting,
    QueueGroup::Unclaimed,
    QueueGroup::SetAside,
    QueueGroup::Done,
];

impl QueueGroup {
    fn label(self) -> &'static str {
        match self {
            Self::NeedsYou => "Needs you",
            Self::Waiting => "Waiting on others",
            Self::Unclaimed => "Unclaimed",
            Self::SetAside => "Set aside",
            Self::Done => "Done",
        }
    }
}

#[derive(Serialize)]
pub(super) struct QueueRow {
    section: RequestQueueSection,
    group: QueueGroup,
    #[serde(flatten)]
    item: RequestQueueItemResponse,
}

/// A maintainer's own request needs them, since merging or closing it is
/// theirs to do. A contributor's own request waits on a maintainer.
fn queue_group(
    section: RequestQueueSection,
    reason: RequestAttentionReason,
    maintainer: bool,
) -> QueueGroup {
    use RequestAttentionReason as Reason;
    match section {
        RequestQueueSection::Unclaimed => QueueGroup::Unclaimed,
        RequestQueueSection::SetAside => QueueGroup::SetAside,
        RequestQueueSection::Done => QueueGroup::Done,
        RequestQueueSection::Active => match reason {
            Reason::Authored if maintainer => QueueGroup::NeedsYou,
            Reason::Invited
            | Reason::Claimed
            | Reason::NewActivity
            | Reason::Restored
            | Reason::SnoozeExpired => QueueGroup::NeedsYou,
            _ => QueueGroup::Waiting,
        },
    }
}

fn reason_label(item: &RequestQueueItemResponse) -> String {
    use RequestAttentionReason as Reason;
    match item.attention.reason {
        Reason::Authored => "Your request".to_string(),
        Reason::Invited => "Review requested".to_string(),
        Reason::Claimed => "You're reviewing".to_string(),
        Reason::Unclaimed => "Waiting for a reviewer".to_string(),
        Reason::NewActivity => "New reply or revision".to_string(),
        Reason::Restored => "Back in your queue".to_string(),
        Reason::SnoozeExpired => "Snooze ended".to_string(),
        Reason::Waiting => "Waiting for a reply".to_string(),
        Reason::Snoozed => match item.attention.snoozed_until_unix {
            Some(until) => format!("Snoozed until {}", local_time_label(until)),
            None => "Snoozed".to_string(),
        },
        Reason::Settled => "Settled for now".to_string(),
        Reason::Open => "Open request".to_string(),
        Reason::ClaimedElsewhere => match &item.claimer {
            Some(claimer) => format!("Reviewing: @{}", terminal_text(&claimer.handle)),
            None => "Being reviewed".to_string(),
        },
        Reason::Closed => "Closed".to_string(),
        Reason::Merged => "Merged".to_string(),
    }
}

fn local_time_label(unix: u64) -> String {
    i64::try_from(unix)
        .ok()
        .and_then(|unix| Local.timestamp_opt(unix, 0).single())
        .map(|time| time.format("%a %b %-d %H:%M").to_string())
        .unwrap_or_else(|| unix.to_string())
}

/// When a snooze preset lands, computed like the web snooze menu: an hour from
/// now, or 09:00 local time tomorrow or next Monday.
fn snooze_until<Tz: TimeZone>(preset: SnoozeFor, now: DateTime<Tz>) -> anyhow::Result<u64> {
    let at_nine = |days: u64| {
        let date = now.date_naive().checked_add_days(Days::new(days))?;
        now.timezone()
            .from_local_datetime(&date.and_time(NaiveTime::from_hms_opt(9, 0, 0)?))
            .earliest()
    };
    let until = match preset {
        SnoozeFor::Hour => Some(now.clone() + chrono::TimeDelta::hours(1)),
        SnoozeFor::Tomorrow => at_nine(1),
        SnoozeFor::NextWeek => match (8 - now.weekday().num_days_from_sunday()) % 7 {
            0 => at_nine(7),
            days => at_nine(days.into()),
        },
    }
    .context("compute the snooze time in the local time zone")?;
    u64::try_from(until.timestamp()).context("snooze time is before the Unix epoch")
}

fn now_unix() -> anyhow::Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before Unix epoch")?
        .as_secs())
}

fn load_queue(
    api: ApiSession<'_>,
    context: &local::RequestContext,
    sections: &[RequestQueueSection],
    search: Option<&str>,
    limit: u32,
) -> anyhow::Result<Vec<QueueRow>> {
    let maintainer = matches!(
        context.repo.access.actor,
        RepositoryActor::Owner | RepositoryActor::Member
    );
    let mut rows = Vec::new();
    for &section in sections {
        let mut remaining = limit as usize;
        let mut cursor = None;
        while remaining > 0 {
            let page = request_queue_page(
                api,
                &context.target.owner,
                &context.target.repo,
                section,
                search,
                cursor.as_deref(),
            )?;
            let taken = page.requests.len().min(remaining);
            remaining -= taken;
            rows.extend(page.requests.into_iter().take(taken).map(|item| QueueRow {
                section,
                group: queue_group(section, item.attention.reason, maintainer),
                item,
            }));
            let Some(next) = page.next_cursor else { break };
            cursor = Some(next);
        }
    }
    Ok(rows)
}

fn queue_lines(rows: &[QueueRow], default_sections: bool, now_unix: u64) -> Vec<String> {
    let mut lines = Vec::new();
    for group in GROUP_ORDER {
        let mut members = rows.iter().filter(|row| row.group == group).peekable();
        if members.peek().is_none() {
            continue;
        }
        lines.push(group.label().to_string());
        lines.extend(members.map(|row| queue_row_line(row, now_unix)));
    }
    if lines.is_empty() {
        lines.push(if default_sections {
            "No requests need you.".to_string()
        } else {
            "No requests in this section.".to_string()
        });
    }
    if default_sections {
        lines.push("More: scope request list --section set-aside | done".to_string());
    }
    lines
}

fn queue_row_line(row: &QueueRow, now_unix: u64) -> String {
    let request = &row.item.request;
    let marker = if row.item.attention.reason == RequestAttentionReason::NewActivity {
        '●'
    } else {
        ' '
    };
    format!(
        "{marker}{:>5}  {} ({}) — {} · {}",
        render::wait_label(Some(row.item.attention_at_unix), now_unix),
        terminal_text(&request.name),
        terminal_text(&request.id),
        terminal_text(&request.title),
        reason_label(&row.item)
    )
}

/// The queue as `request list` and the repository-wide `request status` show it.
pub(super) fn queue_outcome(
    command: &'static str,
    api: ApiSession<'_>,
    context: local::RequestContext,
    section: Option<RequestQueueSection>,
    search: Option<&str>,
    limit: u32,
) -> anyhow::Result<RequestCommandOutcome> {
    let sections = section.map_or(DEFAULT_SECTIONS.to_vec(), |section| vec![section]);
    let rows = load_queue(api, &context, &sections, search, limit)?;
    let mut human_lines = repo_access_lines(&context.repo);
    human_lines.extend(queue_lines(&rows, section.is_none(), now_unix()?));
    Ok(RequestCommandOutcome::new(
        command,
        RequestCommandResult::List(ListResult {
            repo: context.repo,
            requests: rows,
        }),
        human_lines,
    ))
}

pub(super) fn list_request_queue(
    git_repo: Option<&GitRepo>,
    api: ApiSession<'_>,
    args: args::RequestListArgs,
) -> anyhow::Result<RequestCommandOutcome> {
    let context = load_context(git_repo, api, args.remote.as_deref())?;
    queue_outcome(
        "request.list",
        api,
        context,
        args.section.map(Into::into),
        args.search.as_deref(),
        args.limit,
    )
}

#[derive(Clone, Copy)]
pub(super) enum AttentionCommand {
    Claim,
    Release,
    Wait,
    Settle,
    Snooze(SnoozeFor),
    Restore,
}

impl AttentionCommand {
    fn name(self) -> &'static str {
        match self {
            Self::Claim => "request.claim",
            Self::Release => "request.release",
            Self::Wait => "request.wait",
            Self::Settle => "request.settle",
            Self::Snooze(_) => "request.snooze",
            Self::Restore => "request.restore",
        }
    }
}

/// Sends the request's current activity version, so the server only refuses
/// the change when activity lands between this read and the write.
pub(super) fn change_attention(
    git_repo: Option<&GitRepo>,
    api: ApiSession<'_>,
    target: RequestTargetArgs,
    command: AttentionCommand,
) -> anyhow::Result<RequestCommandOutcome> {
    let (context, request_id, detail) = load_exact_request(git_repo, api, target)?;
    let expected_activity_version = detail.request.activity_version;
    let action = match command {
        AttentionCommand::Claim => RequestAttentionActionRequest::Claim {
            expected_activity_version,
        },
        AttentionCommand::Release => RequestAttentionActionRequest::Release {
            expected_activity_version,
        },
        AttentionCommand::Wait => RequestAttentionActionRequest::Wait {
            expected_activity_version,
        },
        AttentionCommand::Settle => RequestAttentionActionRequest::Settle {
            expected_activity_version,
        },
        AttentionCommand::Snooze(preset) => RequestAttentionActionRequest::Snooze {
            expected_activity_version,
            until_unix: snooze_until(preset, Local::now())?,
        },
        AttentionCommand::Restore => RequestAttentionActionRequest::Restore {
            expected_activity_version,
        },
    };
    let name = terminal_text(&detail.request.name);
    let target = api_target(&context, &request_id);
    let response = apply_request_attention(api, target, &action).map_err(|error| {
        explain_stale_attention(api, target, &name, expected_activity_version, error)
    })?;
    let receipt = match command {
        AttentionCommand::Claim => format!("Claimed {name} · you're reviewing it"),
        AttentionCommand::Release => format!("Released {name} · you are no longer its reviewer"),
        AttentionCommand::Wait => format!("Set {name} aside · waiting for a reply"),
        AttentionCommand::Settle => format!("Set {name} aside · settled until new activity"),
        AttentionCommand::Snooze(_) => match response.attention.snoozed_until_unix {
            Some(until) => format!("Snoozed {name} until {}", local_time_label(until)),
            None => format!("Snoozed {name}"),
        },
        AttentionCommand::Restore => format!("Restored {name} · back in Needs you"),
    };
    Ok(RequestCommandOutcome::new(
        command.name(),
        RequestCommandResult::Attention(TargetResponse {
            repo: context.repo,
            request_id,
            response,
        }),
        vec![receipt],
    ))
}

/// A conflict after the version moved means someone acted on the request
/// since it was loaded; show what happened instead of only the refusal.
fn explain_stale_attention(
    api: ApiSession<'_>,
    target: RequestTarget<'_>,
    name: &str,
    sent_version: u64,
    error: anyhow::Error,
) -> anyhow::Error {
    let Some(refusal) = error.downcast_ref::<CliError>() else {
        return error;
    };
    if refusal.response().code != ErrorCode::Conflict {
        return error;
    }
    let Ok(current) = get_request(api, target.owner, target.repo, target.request_id) else {
        return error;
    };
    let current_version = current.request.activity_version;
    if current_version <= sent_version {
        return error;
    }
    let Ok(activity) = full_request_activity(api, target, sent_version, current_version) else {
        return error;
    };
    let mut recovery = vec![format!("{name} has newer activity since it was loaded.")];
    recovery.extend(request_activity_lines_for_response(&activity));
    recovery.push("Run the command again to act on the current state.".to_string());
    CliError::with_recovery(
        refusal.response().clone(),
        serde_json::json!({
            "recovery": recovery.join("\n"),
            "activity": activity,
        }),
    )
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    fn at(rfc3339: &str) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339(rfc3339).unwrap()
    }

    fn unix(rfc3339: &str) -> u64 {
        at(rfc3339).timestamp() as u64
    }

    #[test]
    fn snooze_presets_land_like_the_web_menu() {
        // 2026-10-05 is a Monday.
        let monday = at("2026-10-05T14:30:00-07:00");
        for (preset, expected) in [
            (SnoozeFor::Hour, "2026-10-05T15:30:00-07:00"),
            (SnoozeFor::Tomorrow, "2026-10-06T09:00:00-07:00"),
            (SnoozeFor::NextWeek, "2026-10-12T09:00:00-07:00"),
        ] {
            assert_eq!(snooze_until(preset, monday).unwrap(), unix(expected));
        }
        let sunday = at("2026-10-11T22:00:00+02:00");
        assert_eq!(
            snooze_until(SnoozeFor::NextWeek, sunday).unwrap(),
            unix("2026-10-12T09:00:00+02:00")
        );
    }

    #[test]
    fn rows_show_age_reason_and_new_activity() {
        let item = |reason: &str, claimer: serde_json::Value| -> RequestQueueItemResponse {
            serde_json::from_value(serde_json::json!({
                "attention_at_unix": 1_000,
                "request": {
                    "id": "req_one", "name": "fix-refs", "title": "Fix refs",
                    "author_role": "Public", "audience": "Public", "head_oid": "b".repeat(40),
                    "state": "Open", "submitted_at_unix": 10, "updated_at_unix": 20,
                    "mergeability": {
                        "status": "NotMaintainer",
                        "current_main_oid": "a".repeat(40),
                        "request_head_oid": "b".repeat(40),
                        "reason": "repo maintainer required"
                    }
                },
                "author": null,
                "attention": {
                    "state": "active", "reason": reason, "activity_version": 3,
                    "through_activity_version": 2, "snoozed_until_unix": null, "revision": 1,
                    "can_claim": false, "can_set_aside": true, "can_restore": false,
                    "can_release": false
                },
                "claimer": claimer
            }))
            .unwrap()
        };
        let row = |item| QueueRow {
            section: RequestQueueSection::Active,
            group: QueueGroup::NeedsYou,
            item,
        };

        let fresh = queue_row_line(&row(item("new_activity", serde_json::Value::Null)), 4_600);
        assert_eq!(
            fresh,
            "●   1h  fix-refs (req_one) — Fix refs · New reply or revision"
        );
        let claimed = queue_row_line(
            &row(item(
                "claimed_elsewhere",
                serde_json::json!({"id": "scope_usr_dana", "handle": "dana"}),
            )),
            1_030,
        );
        assert_eq!(
            claimed,
            "   <1m  fix-refs (req_one) — Fix refs · Reviewing: @dana"
        );
    }

    #[test]
    fn groups_follow_the_web_sidebar() {
        use RequestAttentionReason as Reason;
        use RequestQueueSection as Section;
        for (section, reason, maintainer, expected) in [
            (
                Section::Active,
                Reason::Authored,
                true,
                QueueGroup::NeedsYou,
            ),
            (
                Section::Active,
                Reason::Authored,
                false,
                QueueGroup::Waiting,
            ),
            (
                Section::Active,
                Reason::NewActivity,
                true,
                QueueGroup::NeedsYou,
            ),
            (
                Section::Active,
                Reason::SnoozeExpired,
                true,
                QueueGroup::NeedsYou,
            ),
            (
                Section::Active,
                Reason::ClaimedElsewhere,
                true,
                QueueGroup::Waiting,
            ),
            (Section::Active, Reason::Open, false, QueueGroup::Waiting),
            (
                Section::Unclaimed,
                Reason::Unclaimed,
                true,
                QueueGroup::Unclaimed,
            ),
            (
                Section::SetAside,
                Reason::Snoozed,
                true,
                QueueGroup::SetAside,
            ),
            (Section::Done, Reason::Merged, true, QueueGroup::Done),
        ] {
            assert_eq!(queue_group(section, reason, maintainer), expected);
        }
    }
}
