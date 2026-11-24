use super::{
    RequestListPredicate, RequestState, RequestViewer,
    lifecycle::{RequestCloseSubject, ensure_close_allowed},
    request_list_predicate,
};
use crate::views::Views;

pub fn request_backlog_predicate<'a>(
    viewer: &RequestViewer<'a>,
    views: &Views,
) -> RequestListPredicate<'a> {
    RequestListPredicate::All(vec![
        request_list_predicate(viewer.access.clone(), viewer.user_id, views),
        RequestListPredicate::Asked,
        RequestListPredicate::Draft,
    ])
}

pub fn backlog_request_closable(worker_user_id: Option<&str>, viewer: &RequestViewer<'_>) -> bool {
    viewer.user_id.is_some_and(|viewer_user_id| {
        ensure_close_allowed(
            RequestCloseSubject {
                state: RequestState::Draft,
                asked: true,
                actor_is_author: worker_user_id == Some(viewer_user_id),
            },
            viewer.access.is_maintainer(),
        )
        .is_ok()
    })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RequestBacklogWorking {
    #[default]
    Anyone,
    Nobody,
    Me,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RequestBacklogSort {
    #[default]
    Oldest,
    Newest,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestBacklogSearch {
    pub text: Option<String>,
    pub github_issue_number: Option<u64>,
}

impl RequestBacklogSearch {
    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim();
        if value.is_empty() {
            return None;
        }
        if let Some(number) = value.strip_prefix('#').and_then(github_issue_number) {
            return Some(Self {
                text: None,
                github_issue_number: Some(number),
            });
        }
        Some(Self {
            text: Some(value.to_string()),
            github_issue_number: github_issue_number(value),
        })
    }
}

fn github_issue_number(value: &str) -> Option<u64> {
    if !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse().ok().filter(|number| *number > 0)
}

#[cfg(test)]
mod tests;
