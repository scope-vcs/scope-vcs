use crate::api::{
    LeaveRequestResponse, RepoSummaryResponse, RepositoryActor, RequestActivityPageResponse,
    RequestAudience, RequestCheckEvaluationState, RequestCheckResponse, RequestChecksResponse,
    RequestCloseResponse, RequestDiscussionMutationResponse,
    RequestDiscussionReplyMutationResponse, RequestEventPayload, RequestInviteeMutationResponse,
    RequestListItemResponse, RequestMergeabilityResponse, RequestMergeabilityStatus,
    RequestMutationResponse, RequestPermissionsResponse, RequestState, RequestSummaryResponse,
};
use crate::display::{short_oid, terminal_text};

mod auto_merge;
use auto_merge::activity_line as auto_merge_activity_line;
pub(super) use auto_merge::{
    receipt_lines as auto_merge_receipt_lines, status_lines as auto_merge_status_lines,
};

pub(super) fn repo_access_lines(repo: &RepoSummaryResponse) -> Vec<String> {
    vec![
        format!("Scope repo: {}/{}", repo.owner_handle, repo.name),
        format!("Permission: {}", access_label(repo.access.actor)),
    ]
}

pub(super) fn request_activity_lines_for_response(
    activity: &RequestActivityPageResponse,
) -> Vec<String> {
    let lines = request_activity_lines(activity);
    if lines.is_empty() {
        return Vec::new();
    }
    let mut rendered = vec!["Activity:".to_string()];
    for line in lines {
        rendered.push(format!("  {line}"));
    }
    rendered
}

pub(super) fn request_mutation_receipt_lines(
    action: &str,
    response: &RequestMutationResponse,
) -> Vec<String> {
    let action = terminal_text(action);
    vec![format!("{action} · {}", request_line(&response.request))]
}

pub(super) fn invitee_added_receipt(response: &RequestInviteeMutationResponse) -> String {
    format!(
        "Invited @{} · can now push request {}",
        terminal_text(&response.invitee.user.handle),
        response.request.name
    )
}

pub(super) fn invitee_removed_receipt(response: &RequestInviteeMutationResponse) -> String {
    format!(
        "Removed @{} from request {}",
        terminal_text(&response.invitee.user.handle),
        response.request.name
    )
}

pub(super) fn leave_receipt(request_id: &str, response: &LeaveRequestResponse) -> String {
    format!(
        "@{} left request {}",
        terminal_text(&response.invitee.user.handle),
        request_id
    )
}

pub(super) fn close_receipt(request_id: &str, response: &RequestCloseResponse) -> String {
    if response.deleted {
        format!("Closed and removed draft request {request_id}")
    } else if let Some(request) = response.request.as_ref() {
        format!("Closed request {} · remains in history", request.name)
    } else {
        format!("Closed request {request_id}")
    }
}

pub(super) fn discussion_started_receipt(
    request_id: &str,
    response: &RequestDiscussionMutationResponse,
) -> Vec<String> {
    let discussion = &response.discussion;
    let mut lines = vec![format!(
        "Created discussion {} on request {}",
        terminal_text(&discussion.id),
        terminal_text(request_id),
    )];
    if let Some(anchor) = discussion.anchor.as_ref() {
        let mut reference = format!("Revision {}", terminal_text(&anchor.revision_id));
        if let Some(commit) = anchor.commit_oid.as_deref() {
            reference.push_str(&format!(" · commit {}", short_oid(commit)));
        }
        if let Some(path) = anchor.path.as_deref() {
            reference.push_str(&format!(" · {}", terminal_text(path)));
        }
        lines.push(reference);
    }
    lines
}

pub(super) fn discussion_replied_receipt(
    response: &RequestDiscussionReplyMutationResponse,
) -> String {
    format!(
        "Replied to discussion {} · reply {}",
        terminal_text(&response.discussion.id),
        terminal_text(&response.reply.id),
    )
}

pub(super) fn discussion_resolved_receipt(response: &RequestDiscussionMutationResponse) -> String {
    format!(
        "Resolved discussion {}",
        terminal_text(&response.discussion.id),
    )
}

pub(super) fn discussion_reopened_receipt(
    response: &RequestDiscussionReplyMutationResponse,
) -> String {
    format!(
        "Reopened discussion {} · reply {}",
        terminal_text(&response.discussion.id),
        terminal_text(&response.reply.id),
    )
}

pub(super) fn request_line(request: &RequestSummaryResponse) -> String {
    format_request_line(RequestLine {
        name: &request.name,
        id: &request.id,
        state: request.state,
        title: &request.title,
        head_oid: &request.head_oid,
    })
}

pub(super) fn request_list_line(request: &RequestListItemResponse, now_unix: u64) -> String {
    format!(
        "{:>5}  {}",
        wait_label(request.submitted_at_unix, now_unix),
        format_request_line(RequestLine {
            name: &request.name,
            id: &request.id,
            state: request.state,
            title: &request.title,
            head_oid: &request.head_oid,
        })
    )
}

fn wait_label(submitted_at_unix: Option<u64>, now_unix: u64) -> String {
    let Some(submitted_at_unix) = submitted_at_unix else {
        return "-".to_string();
    };
    let seconds = now_unix.saturating_sub(submitted_at_unix);
    if seconds < 60 {
        "<1m".to_string()
    } else if seconds < 60 * 60 {
        format!("{}m", seconds / 60)
    } else if seconds < 24 * 60 * 60 {
        format!("{}h", seconds / (60 * 60))
    } else {
        format!("{}d", seconds / (24 * 60 * 60))
    }
}

pub(super) fn request_detail_lines(request: &RequestSummaryResponse) -> Vec<String> {
    let mut lines = vec![
        request_line(request),
        format!(
            "  lifecycle: {} · {}",
            state_label(request.state),
            if request.submitted_at_unix.is_some() {
                "submitted"
            } else {
                "not yet submitted"
            }
        ),
        format!(
            "  branch: {} · base {} {} · head {}",
            request.name,
            audience_label(request.audience),
            short_oid(&request.base_main_oid),
            short_oid(&request.head_oid)
        ),
    ];
    if !request.description_markdown.trim().is_empty() {
        lines.push(format!(
            "  description: {}",
            terminal_text(request.description_markdown.trim())
        ));
    }
    lines.push(if request.invitees.is_empty() {
        "  invitees: none".to_string()
    } else {
        format!(
            "  invitees: {}",
            request
                .invitees
                .iter()
                .map(|invitee| format!("@{}", terminal_text(&invitee.user.handle)))
                .collect::<Vec<_>>()
                .join(", ")
        )
    });
    lines.push(format!(
        "  capabilities: {}",
        capabilities_label(&request.permissions)
    ));
    lines.push(format!(
        "  mergeability: {}",
        mergeability_label(&request.mergeability)
    ));
    if let Some(merged_at) = request.merged_at_unix {
        lines.push(format!(
            "  merge: {} → {} · at {merged_at}",
            request
                .merged_head_oid
                .as_deref()
                .map(short_oid)
                .unwrap_or_else(|| short_oid(&request.head_oid)),
            request
                .merged_main_oid
                .as_deref()
                .map(short_oid)
                .unwrap_or("unknown")
        ));
    }
    lines
}

fn request_activity_lines(activity: &RequestActivityPageResponse) -> Vec<String> {
    let mut events = activity.events.iter().collect::<Vec<_>>();
    events.sort_by_key(|event| event.position);
    let mut lines = Vec::new();
    for event in events {
        let action = match &event.payload {
            RequestEventPayload::Started { .. } => "Started request".to_string(),
            RequestEventPayload::Submitted { head_oid } => {
                format!("Submitted · head {}", short_oid(head_oid))
            }
            RequestEventPayload::RevisionPushed {
                old_head_oid,
                new_head_oid,
                note,
            } => {
                let mut line = format!(
                    "Revision pushed · {}..{}",
                    short_oid(old_head_oid),
                    short_oid(new_head_oid)
                );
                if let Some(note) = note {
                    line.push_str(&format!(" · {}", terminal_text(note)));
                }
                line
            }
            RequestEventPayload::Merged { head_oid, main_oid } => format!(
                "Merged · head {} · main {}",
                short_oid(head_oid),
                short_oid(main_oid)
            ),
            RequestEventPayload::Closed { head_oid } => {
                format!("Closed · head {}", short_oid(head_oid))
            }
            RequestEventPayload::IdentityEdited { .. } => "Edited title or description".to_string(),
            RequestEventPayload::DiscussionResolved { discussion_id } => {
                format!("Resolved discussion {}", terminal_text(discussion_id))
            }
            RequestEventPayload::DiscussionReopened { discussion_id } => {
                format!("Reopened discussion {}", terminal_text(discussion_id))
            }
            payload @ (RequestEventPayload::AutoMergeEnabled { .. }
            | RequestEventPayload::AutoMergeCancelled { .. }
            | RequestEventPayload::AutoMergeStopped { .. }
            | RequestEventPayload::AutoMergeFulfilled { .. }) => auto_merge_activity_line(payload),
        };
        lines.push(format!("{action} · at {}", event.created_at_unix));
    }
    lines
}

struct RequestLine<'a> {
    name: &'a str,
    id: &'a str,
    state: RequestState,
    title: &'a str,
    head_oid: &'a str,
}

fn format_request_line(line: RequestLine<'_>) -> String {
    format!(
        "{:<9}  {} ({}) — {} · head {}",
        state_label(line.state),
        terminal_text(line.name),
        terminal_text(line.id),
        terminal_text(line.title),
        short_oid(line.head_oid)
    )
}

fn capabilities_label(permissions: &RequestPermissionsResponse) -> String {
    let capabilities = [
        (permissions.can_push_branch, "push"),
        (permissions.can_pull_branch, "pull"),
        (permissions.can_submit, "submit"),
        (permissions.can_edit_identity, "edit"),
        (permissions.can_manage_invitees, "invitees"),
        (permissions.can_leave_request, "leave"),
        (permissions.can_merge, "merge"),
        (permissions.can_close, "close"),
        (permissions.can_open_discussion, "discuss"),
        (permissions.can_reply_to_discussion, "reply"),
    ]
    .into_iter()
    .filter_map(|(allowed, label)| allowed.then_some(label))
    .collect::<Vec<_>>();
    if capabilities.is_empty() {
        "view only".to_string()
    } else {
        capabilities.join(", ")
    }
}

/// The checks the request head asks for, and what merging still waits on.
pub(super) fn request_checks_lines(checks: &RequestChecksResponse) -> Vec<String> {
    let mut lines = vec![
        format!(
            "Request {}, head {}",
            terminal_text(&checks.request_id),
            short_oid(checks.head_oid.as_str())
        ),
        format!("Checks: {}", evaluation_state_label(checks.state)),
    ];
    if let Some(message) = &checks.message {
        lines.push(format!("  {}", terminal_text(message)));
    }
    if checks.state == Some(RequestCheckEvaluationState::NoChecks) {
        lines.push("  This head asks for no checks.".to_string());
    }
    for check in &checks.checks {
        lines.push(format!(
            "  {} · {}",
            terminal_text(&check.workflow_name),
            check_run_label(check)
        ));
    }
    lines.push(format!(
        "Mergeability: {}",
        mergeability_label(&checks.mergeability)
    ));
    if checks.can_approve {
        lines.push("Start these checks with `scope request checks --approve`.".to_string());
    }
    lines
}

fn evaluation_state_label(state: Option<RequestCheckEvaluationState>) -> &'static str {
    match state {
        None => "not worked out for this commit yet",
        Some(RequestCheckEvaluationState::NoChecks) => "none asked for",
        Some(RequestCheckEvaluationState::AwaitingApproval) => "waiting for maintainer approval",
        Some(RequestCheckEvaluationState::Started) => "started",
        Some(RequestCheckEvaluationState::ConfigurationError) => "workflow configuration error",
    }
}

fn check_run_label(check: &RequestCheckResponse) -> String {
    match (&check.run_id, check.run_state) {
        (Some(run_id), Some(state)) => format!(
            "{} ({})",
            crate::run::run_state_label(state),
            terminal_text(run_id)
        ),
        (Some(run_id), None) => format!("run is gone ({})", terminal_text(run_id)),
        (None, _) => "not started".to_string(),
    }
}

fn mergeability_label(mergeability: &RequestMergeabilityResponse) -> String {
    match mergeability.status {
        RequestMergeabilityStatus::Ready => "ready".to_string(),
        RequestMergeabilityStatus::Closed => "closed".to_string(),
        RequestMergeabilityStatus::Merged => "merged".to_string(),
        RequestMergeabilityStatus::Draft => "draft".to_string(),
        RequestMergeabilityStatus::NotMaintainer => mergeability
            .reason
            .clone()
            .unwrap_or_else(|| "repo maintainer required".to_string()),
        RequestMergeabilityStatus::MissingRequestBranch => mergeability
            .reason
            .clone()
            .unwrap_or_else(|| "request branch has not been pushed".to_string()),
        RequestMergeabilityStatus::ChecksNotEvaluated
        | RequestMergeabilityStatus::ChecksAwaitingApproval
        | RequestMergeabilityStatus::ChecksPending
        | RequestMergeabilityStatus::ChecksFailed
        | RequestMergeabilityStatus::ChecksConfigurationError => mergeability
            .reason
            .clone()
            .unwrap_or_else(|| "checks have not passed".to_string()),
    }
}

fn access_label(actor: RepositoryActor) -> &'static str {
    match actor {
        RepositoryActor::Owner => "owner",
        RepositoryActor::Member => "member",
        RepositoryActor::Public => "public contributor",
    }
}

pub(super) fn audience_label(audience: RequestAudience) -> &'static str {
    match audience {
        RequestAudience::Public => "public main",
        RequestAudience::Private => "private main",
    }
}

fn state_label(state: RequestState) -> &'static str {
    match state {
        RequestState::Draft => "draft",
        RequestState::Open => "open",
        RequestState::Closed => "closed",
        RequestState::Merged => "merged",
    }
}

#[cfg(test)]
#[path = "render_tests.rs"]
mod tests;
