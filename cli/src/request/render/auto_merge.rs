use crate::{
    api::{
        RequestAutoMergeIntentStatus, RequestAutoMergeResponse, RequestAutoMergeStopReason,
        RequestEventPayload,
    },
    display::{short_oid, terminal_text},
};

pub(in crate::request) fn receipt_lines(
    action: &str,
    response: &RequestAutoMergeResponse,
) -> Vec<String> {
    let mut lines = vec![format!(
        "{} · request {} · head {}",
        terminal_text(action),
        terminal_text(&response.request_id),
        short_oid(response.head_oid.as_str())
    )];
    match response.intent.as_ref().map(|intent| intent.status) {
        Some(RequestAutoMergeIntentStatus::Fulfilled) => {
            lines.push("Request merged automatically.".to_string());
        }
        Some(RequestAutoMergeIntentStatus::Active) => {
            lines.push("Authorization saved; the request has not merged yet.".to_string());
        }
        Some(RequestAutoMergeIntentStatus::Cancelled) => {
            lines.push(
                "Auto-merge is canceled; passing checks will leave the request open.".to_string(),
            );
        }
        Some(RequestAutoMergeIntentStatus::Stopped) => {
            lines.push("Auto-merge stopped without merging the request.".to_string());
        }
        None => {
            lines.push("Auto-merge is not enabled.".to_string());
        }
    }
    if let Some(reason) = response.waiting_reason.as_deref() {
        lines.push(format!("Waiting: {}", terminal_text(reason)));
    } else if let Some(reason) = response.intent.as_ref().and_then(|intent| intent.reason) {
        lines.push(format!("Reason: {}", stop_reason_label(reason)));
    }
    lines
}

pub(in crate::request) fn status_lines(response: &RequestAutoMergeResponse) -> Vec<String> {
    let Some(intent) = response.intent.as_ref() else {
        return vec!["  auto-merge: not enabled".to_string()];
    };
    let mut lines = vec![format!(
        "  auto-merge: {:?} · authorized by @{} · head {}",
        intent.status,
        terminal_text(&intent.actor.handle),
        short_oid(intent.head_oid.as_str())
    )];
    if let Some(reason) = response.waiting_reason.as_deref() {
        lines.push(format!("  auto-merge waiting: {}", terminal_text(reason)));
    } else if let Some(reason) = intent.reason {
        lines.push(format!(
            "  auto-merge reason: {}",
            stop_reason_label(reason)
        ));
    }
    lines
}

pub(super) fn stop_reason_label(reason: RequestAutoMergeStopReason) -> &'static str {
    match reason {
        RequestAutoMergeStopReason::RequestChanged => "the request changed",
        RequestAutoMergeStopReason::RequestClosed => "the request was closed",
        RequestAutoMergeStopReason::AccessRevoked => "the authorizer lost maintainer access",
        RequestAutoMergeStopReason::ChecksFailed => "checks failed",
        RequestAutoMergeStopReason::ChecksConfigurationError => "checks have a configuration error",
        RequestAutoMergeStopReason::MergeConflict => "the request conflicts with main",
        RequestAutoMergeStopReason::RequestBranchMissing => "the request branch is missing",
    }
}

pub(super) fn activity_line(payload: &RequestEventPayload) -> String {
    match payload {
        RequestEventPayload::AutoMergeEnabled { head_oid, .. } => {
            format!("Enabled auto-merge · head {}", short_oid(head_oid))
        }
        RequestEventPayload::AutoMergeCancelled { head_oid, .. } => {
            format!("Canceled auto-merge · head {}", short_oid(head_oid))
        }
        RequestEventPayload::AutoMergeStopped {
            head_oid, reason, ..
        } => format!(
            "Stopped auto-merge · head {} · {}",
            short_oid(head_oid),
            stop_reason_label(*reason)
        ),
        RequestEventPayload::AutoMergeFulfilled {
            head_oid, main_oid, ..
        } => format!(
            "Fulfilled auto-merge · head {} · main {}",
            short_oid(head_oid),
            short_oid(main_oid)
        ),
        _ => unreachable!("only auto-merge activity is rendered here"),
    }
}
