use crate::display::{short_oid, terminal_text};
use scope_api_contract::{
    RequestBase, RequestStackNavigationResponse, RequestStackParentResponse, RequestSummaryResponse,
};

pub(in crate::request) fn parent_label(parent: Option<&RequestStackParentResponse>) -> String {
    match parent {
        Some(RequestStackParentResponse::Available { request }) => terminal_text(&request.name),
        Some(RequestStackParentResponse::Unavailable) => "dependency unavailable".to_string(),
        None => "main".to_string(),
    }
}

pub(super) fn lines(
    request: &RequestSummaryResponse,
    navigation: &RequestStackNavigationResponse,
) -> Vec<String> {
    let mut lines = Vec::new();
    if !navigation.ancestors.is_empty() || navigation.ancestors_unavailable {
        let mut path = Vec::new();
        if navigation.ancestors_unavailable {
            path.push("dependency unavailable".to_string());
        }
        path.extend(
            navigation
                .ancestors
                .iter()
                .map(|node| terminal_text(&node.name)),
        );
        path.push(terminal_text(&request.name));
        lines.push(format!("  path: {}", path.join(" → ")));
    }
    lines.push(match &request.base {
        RequestBase::Main { review_base_oid } => {
            format!("  review base: main @ {}", short_oid(review_base_oid))
        }
        RequestBase::Request {
            revision_id,
            head_oid,
            ..
        } => format!(
            "  review pin: {} @ {} ({})",
            parent_label(navigation.parent.as_ref()),
            terminal_text(revision_id),
            short_oid(head_oid)
        ),
    });
    if let Some(RequestStackParentResponse::Available { request: parent }) = &navigation.parent {
        lines.push(format!(
            "  parent current: {} @ {} ({})",
            terminal_text(&parent.name),
            parent
                .current_revision_id
                .as_deref()
                .map(terminal_text)
                .unwrap_or_else(|| "no saved revision".to_string()),
            short_oid(&parent.head_oid)
        ));
    }
    if !navigation.children.requests.is_empty() {
        lines.push(format!(
            "  children: {}",
            navigation
                .children
                .requests
                .iter()
                .map(|child| format!(
                    "{} ({}) · {:?}",
                    terminal_text(&child.name),
                    terminal_text(&child.id),
                    child.stack_status
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if navigation.children.next_cursor.is_some() {
        lines.push("  more children: open this request's web page".to_string());
    }
    lines
}
