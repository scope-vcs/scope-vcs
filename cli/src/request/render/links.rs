use crate::api::{
    LinkedRequestResponse, RequestCloseReason, RequestDetailResponse, RequestMergeabilityStatus,
};
use crate::display::terminal_text;

pub(in crate::request) fn links_lines(detail: &RequestDetailResponse) -> Vec<String> {
    let linked = |label: &str, linked: &LinkedRequestResponse| {
        let mut line = format!(
            "  {label}: {} ({})",
            terminal_text(&linked.title),
            terminal_text(&linked.id)
        );
        if let Some(author) = &linked.author {
            line.push_str(&format!(" · @{}", terminal_text(&author.handle)));
        }
        line.push_str(&format!(" · {}", linked_status_label(linked.status)));
        line
    };
    let links = &detail.links;
    let mut lines = links
        .resolves
        .iter()
        .map(|resolved| linked("resolves", resolved))
        .chain(
            links
                .resolved_by
                .iter()
                .map(|resolving| linked("resolved by", resolving)),
        )
        .collect::<Vec<_>>();
    lines.extend(
        detail
            .request
            .close_reason
            .map(|reason| match (reason, &links.close_named) {
                (RequestCloseReason::Completed, Some(fixed_by)) => {
                    linked("closed as completed, fixed by", fixed_by)
                }
                (RequestCloseReason::Duplicate, Some(duplicated)) => {
                    linked("closed as a duplicate of", duplicated)
                }
                (RequestCloseReason::Completed, None) => "  closed as completed".to_string(),
                (RequestCloseReason::Duplicate, None) => "  closed as a duplicate".to_string(),
                (RequestCloseReason::NotPlanned, _) => "  closed as not planned".to_string(),
            }),
    );
    lines
}

fn linked_status_label(status: RequestMergeabilityStatus) -> &'static str {
    match status {
        RequestMergeabilityStatus::Ready => "ready to merge",
        RequestMergeabilityStatus::Closed => "closed",
        RequestMergeabilityStatus::Merged => "merged",
        RequestMergeabilityStatus::Draft => "draft",
        RequestMergeabilityStatus::NoChangeYet => "no change yet",
        RequestMergeabilityStatus::RepositoryNotReady
        | RequestMergeabilityStatus::NotMaintainer
        | RequestMergeabilityStatus::MissingRequestBranch
        | RequestMergeabilityStatus::NeedsMainUpdate
        | RequestMergeabilityStatus::WaitingForParent
        | RequestMergeabilityStatus::ParentRewritten
        | RequestMergeabilityStatus::ParentClosed
        | RequestMergeabilityStatus::AncestorBlocked
        | RequestMergeabilityStatus::IntegrationConflict
        | RequestMergeabilityStatus::ChecksNotEvaluated
        | RequestMergeabilityStatus::ChecksAwaitingApproval
        | RequestMergeabilityStatus::ChecksPending
        | RequestMergeabilityStatus::ChecksFailed
        | RequestMergeabilityStatus::ChecksConfigurationError
        | RequestMergeabilityStatus::ReviewsPending
        | RequestMergeabilityStatus::ReviewFindingsOpen => "open",
    }
}
