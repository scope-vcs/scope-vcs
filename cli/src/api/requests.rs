use super::*;
use crate::api::ApiSession;
use anyhow::Context;
use reqwest::blocking::RequestBuilder;
use serde::de::DeserializeOwned;

#[derive(Clone, Copy)]
pub struct RequestTarget<'a> {
    pub owner: &'a str,
    pub repo: &'a str,
    pub request_id: &'a str,
}

pub struct StartRequestParams<'a> {
    pub base: Option<RequestBase>,
    pub owner: &'a str,
    pub repo: &'a str,
    pub name: String,
    pub title: Option<String>,
    pub view: ViewId,
}

pub struct CreateRequestDiscussionParams<'a> {
    pub target: RequestTarget<'a>,
    pub body_markdown: String,
    pub client_discussion_id: String,
    pub anchor: Option<RequestDiscussionAnchorInput>,
}

pub struct CreateRequestDiscussionReplyParams<'a> {
    pub target: RequestTarget<'a>,
    pub discussion_id: &'a str,
    pub body_markdown: String,
    pub client_reply_id: String,
}

pub struct RequestActivityParams<'a> {
    pub target: RequestTarget<'a>,
    pub after: Option<u64>,
    pub latest: bool,
    pub limit: Option<usize>,
}

pub fn list_requests(
    api: ApiSession<'_>,
    owner: &str,
    repo: &str,
    cursor: Option<&str>,
) -> anyhow::Result<RequestListResponse> {
    let mut request = api.request(reqwest::Method::GET, routes::repo_requests(owner, repo));
    if let Some(cursor) = cursor {
        request = request.query(&[("cursor", cursor)]);
    }
    execute(request, format!("list requests for {owner}/{repo}"))
}

pub fn request_named(
    api: ApiSession<'_>,
    owner: &str,
    repo: &str,
    name: &str,
) -> anyhow::Result<Option<RequestListItemResponse>> {
    let page: RequestListResponse = execute(
        api.request(reqwest::Method::GET, routes::repo_requests(owner, repo))
            .query(&[("name", name)]),
        format!("find request {name} in {owner}/{repo}"),
    )?;
    Ok(page
        .requests
        .into_iter()
        .find(|request| request.name == name))
}

#[derive(serde::Serialize)]
pub struct QueuePageQuery<'a> {
    pub section: RequestQueueSection,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<&'a str>,
    pub limit: u32,
}

pub fn request_queue_page(
    api: ApiSession<'_>,
    owner: &str,
    repo: &str,
    query: QueuePageQuery<'_>,
) -> anyhow::Result<RequestQueuePageResponse> {
    execute(
        api.request(
            reqwest::Method::GET,
            routes::repo_request_queue(owner, repo),
        )
        .query(&query),
        format!("load the request queue for {owner}/{repo}"),
    )
}

pub fn apply_request_attention(
    api: ApiSession<'_>,
    target: RequestTarget<'_>,
    action: &RequestAttentionActionRequest,
) -> anyhow::Result<RequestAttentionMutationResponse> {
    execute(
        api.request(
            reqwest::Method::PUT,
            routes::repo_request_attention(target.owner, target.repo, target.request_id),
        )
        .json(action),
        format!(
            "change attention for request {} in {}/{}",
            target.request_id, target.owner, target.repo
        ),
    )
}

pub fn get_request(
    api: ApiSession<'_>,
    owner: &str,
    repo: &str,
    request_id: &str,
) -> anyhow::Result<RequestDetailResponse> {
    let target = RequestTarget {
        owner,
        repo,
        request_id,
    };
    execute(
        api.request(reqwest::Method::GET, request_path(target)),
        format!(
            "load request {} for {}/{}",
            target.request_id, target.owner, target.repo
        ),
    )
}

pub fn request_revisions(
    api: ApiSession<'_>,
    target: RequestTarget<'_>,
    revision: Option<&str>,
    commit: Option<&str>,
    comparison: Option<&str>,
) -> anyhow::Result<RequestRevisionListResponse> {
    let mut request = api.request(
        reqwest::Method::GET,
        routes::repo_request_revisions(target.owner, target.repo, target.request_id),
    );
    if let Some(comparison) = comparison {
        request = request.query(&[("comparison", comparison)]);
    }
    if let Some(revision) = revision {
        request = request.query(&[("revision", revision)]);
    }
    if let Some(commit) = commit {
        request = request.query(&[("commit", commit)]);
    }
    execute(
        request,
        format!(
            "inspect request revisions {} for {}/{}",
            target.request_id, target.owner, target.repo
        ),
    )
}

pub struct RequestFileDiffParams<'a> {
    pub target: RequestTarget<'a>,
    pub revision: &'a str,
    pub commit: Option<&'a str>,
    pub comparison: &'a str,
    pub path: &'a str,
}

pub fn request_file_diff(
    api: ApiSession<'_>,
    params: RequestFileDiffParams<'_>,
) -> anyhow::Result<ReviewFileDiffResponse> {
    execute(
        api.request(
            reqwest::Method::GET,
            match params.commit {
                Some(commit) => routes::repo_request_revision_commit_file_diff(
                    params.target.owner,
                    params.target.repo,
                    params.target.request_id,
                    params.revision,
                    commit,
                ),
                None => routes::repo_request_revision_file_diff(
                    params.target.owner,
                    params.target.repo,
                    params.target.request_id,
                    params.revision,
                ),
            },
        )
        .query(&[("path", params.path), ("comparison", params.comparison)]),
        format!(
            "inspect request file diff {} for {}/{}",
            params.target.request_id, params.target.owner, params.target.repo
        ),
    )
}

pub fn request_checks(
    api: ApiSession<'_>,
    target: RequestTarget<'_>,
) -> anyhow::Result<RequestChecksResponse> {
    execute(
        api.request(
            reqwest::Method::GET,
            routes::repo_request_checks(target.owner, target.repo, target.request_id),
        ),
        format!(
            "load request CI {} for {}/{}",
            target.request_id, target.owner, target.repo
        ),
    )
}

pub fn approve_request_checks(
    api: ApiSession<'_>,
    target: RequestTarget<'_>,
    expected_head_oid: GitOid,
) -> anyhow::Result<RequestChecksResponse> {
    execute(
        api.request(
            reqwest::Method::POST,
            routes::repo_request_checks_approve(target.owner, target.repo, target.request_id),
        )
        .json(&ApproveRequestChecksRequest { expected_head_oid }),
        format!(
            "allow request CI {} for {}/{}",
            target.request_id, target.owner, target.repo
        ),
    )
}

pub fn close_request(
    api: ApiSession<'_>,
    owner: &str,
    repo: &str,
    request_id: &str,
) -> anyhow::Result<RequestCloseResponse> {
    let target = RequestTarget {
        owner,
        repo,
        request_id,
    };
    execute(
        api.request(reqwest::Method::DELETE, request_path(target)),
        format!(
            "close request {} for {}/{}",
            target.request_id, target.owner, target.repo
        ),
    )
}

pub fn start_request(
    api: ApiSession<'_>,
    params: StartRequestParams<'_>,
) -> anyhow::Result<RequestMutationResponse> {
    let owner = params.owner;
    let repo = params.repo;
    execute(
        api.request(reqwest::Method::POST, routes::repo_requests(owner, repo))
            .json(&StartRequestRequest {
                base: params.base,
                name: params.name,
                title: params.title,
                view: params.view,
            }),
        format!("start request for {owner}/{repo}"),
    )
}

pub fn request_revision_context(
    api: ApiSession<'_>,
    target: RequestTarget<'_>,
    body: &RequestRevisionContextRequest,
) -> anyhow::Result<RequestRevisionContextResponse> {
    execute(
        api.request(
            reqwest::Method::POST,
            routes::repo_request_revision_context(target.owner, target.repo, target.request_id),
        )
        .json(body),
        format!("publish review context for request {}", target.request_id),
    )
}

pub fn submit_request(
    api: ApiSession<'_>,
    target: RequestTarget<'_>,
) -> anyhow::Result<RequestMutationResponse> {
    execute(
        api.request(reqwest::Method::POST, request_action_path(target, "submit"))
            .json(&SubmitRequestRequest {}),
        format!(
            "submit request {} for {}/{}",
            target.request_id, target.owner, target.repo
        ),
    )
}

pub fn merge_request(
    api: ApiSession<'_>,
    target: RequestTarget<'_>,
    expected_head_oid: GitOid,
) -> anyhow::Result<RequestMutationResponse> {
    execute(
        api.request(reqwest::Method::POST, request_action_path(target, "merge"))
            .json(&MergeRequestRequest { expected_head_oid }),
        format!(
            "merge request {} for {}/{}",
            target.request_id, target.owner, target.repo
        ),
    )
}

pub fn get_request_auto_merge(
    api: ApiSession<'_>,
    target: RequestTarget<'_>,
) -> anyhow::Result<RequestAutoMergeResponse> {
    execute(
        api.request(
            reqwest::Method::GET,
            routes::repo_request_auto_merge(target.owner, target.repo, target.request_id),
        ),
        format!(
            "load automatic merge status for request {} in {}/{}",
            target.request_id, target.owner, target.repo
        ),
    )
}

pub fn authorize_request_auto_merge(
    api: ApiSession<'_>,
    target: RequestTarget<'_>,
    expected_revision_id: String,
    expected_head_oid: GitOid,
) -> anyhow::Result<RequestAutoMergeResponse> {
    execute(
        api.request(
            reqwest::Method::POST,
            routes::repo_request_auto_merge(target.owner, target.repo, target.request_id),
        )
        .json(&AuthorizeRequestAutoMergeRequest {
            expected_revision_id,
            expected_head_oid,
        }),
        format!(
            "authorize automatic merge for request {} in {}/{}",
            target.request_id, target.owner, target.repo
        ),
    )
}

pub fn cancel_request_auto_merge(
    api: ApiSession<'_>,
    target: RequestTarget<'_>,
    expected_intent_id: String,
) -> anyhow::Result<RequestAutoMergeResponse> {
    execute(
        api.request(
            reqwest::Method::DELETE,
            routes::repo_request_auto_merge(target.owner, target.repo, target.request_id),
        )
        .json(&CancelRequestAutoMergeRequest { expected_intent_id }),
        format!(
            "cancel automatic merge for request {} in {}/{}",
            target.request_id, target.owner, target.repo
        ),
    )
}

pub fn rate_request(
    api: ApiSession<'_>,
    target: RequestTarget<'_>,
    score: u8,
    reason: String,
) -> anyhow::Result<RequestRatingResponse> {
    execute(
        api.request(
            reqwest::Method::POST,
            request_action_path(target, "ratings"),
        )
        .json(&CreateRequestRatingRequest { score, reason }),
        format!(
            "rate request participant {} for {}/{}",
            target.request_id, target.owner, target.repo
        ),
    )
}

pub fn edit_request_identity(
    api: ApiSession<'_>,
    target: RequestTarget<'_>,
    title: Option<String>,
    description_markdown: Option<String>,
    expected_description_markdown: Option<String>,
) -> anyhow::Result<RequestMutationResponse> {
    execute(
        api.request(reqwest::Method::PATCH, request_path(target))
            .json(&EditRequestIdentityRequest {
                title,
                description_markdown,
                expected_description_markdown,
            }),
        format!(
            "edit request identity {} for {}/{}",
            target.request_id, target.owner, target.repo
        ),
    )
}

pub fn add_request_invitee(
    api: ApiSession<'_>,
    target: RequestTarget<'_>,
    handle: String,
) -> anyhow::Result<RequestInviteeMutationResponse> {
    execute(
        api.request(
            reqwest::Method::PUT,
            request_action_path(target, "invitees"),
        )
        .json(&AddRequestInviteeRequest { handle }),
        format!(
            "invite request collaborator {} for {}/{}",
            target.request_id, target.owner, target.repo
        ),
    )
}

pub fn remove_request_invitee(
    api: ApiSession<'_>,
    target: RequestTarget<'_>,
    handle: String,
) -> anyhow::Result<RequestInviteeMutationResponse> {
    execute(
        api.request(
            reqwest::Method::DELETE,
            request_action_path(target, "invitees"),
        )
        .json(&RemoveRequestInviteeRequest { handle }),
        format!(
            "remove request invitee {} for {}/{}",
            target.request_id, target.owner, target.repo
        ),
    )
}

pub fn leave_request(
    api: ApiSession<'_>,
    target: RequestTarget<'_>,
) -> anyhow::Result<LeaveRequestResponse> {
    execute(
        api.request(
            reqwest::Method::DELETE,
            routes::repo_request_invitees_me(target.owner, target.repo, target.request_id),
        ),
        format!(
            "leave request {} for {}/{}",
            target.request_id, target.owner, target.repo
        ),
    )
}

pub fn get_request_activity(
    api: ApiSession<'_>,
    params: RequestActivityParams<'_>,
) -> anyhow::Result<RequestActivityPageResponse> {
    let mut request = api.request(
        reqwest::Method::GET,
        request_action_path(params.target, "activity"),
    );
    if let Some(after) = params.after {
        request = request.query(&[("after", after)]);
    }
    if params.latest {
        request = request.query(&[("latest", true)]);
    }
    if let Some(limit) = params.limit {
        request = request.query(&[("limit", limit)]);
    }
    execute(
        request,
        format!(
            "load request activity {} for {}/{}",
            params.target.request_id, params.target.owner, params.target.repo
        ),
    )
}

pub fn create_request_discussion(
    api: ApiSession<'_>,
    params: CreateRequestDiscussionParams<'_>,
) -> anyhow::Result<RequestDiscussionMutationResponse> {
    let target = params.target;
    execute(
        api.request(
            reqwest::Method::POST,
            request_action_path(target, "timeline"),
        )
        .json(&CreateRequestDiscussionRequest {
            body_markdown: params.body_markdown,
            client_discussion_id: params.client_discussion_id,
            anchor: params.anchor,
        }),
        format!(
            "create request discussion {} for {}/{}",
            target.request_id, target.owner, target.repo
        ),
    )
}

pub fn create_request_discussion_reply(
    api: ApiSession<'_>,
    params: CreateRequestDiscussionReplyParams<'_>,
) -> anyhow::Result<RequestDiscussionReplyMutationResponse> {
    execute(
        api.request(
            reqwest::Method::POST,
            request_discussion_action_path(params.target, params.discussion_id, "replies"),
        )
        .json(&CreateRequestDiscussionReplyRequest {
            body_markdown: params.body_markdown,
            client_reply_id: params.client_reply_id,
            reply_to_reply_id: None,
            wait_after_reply: false,
        }),
        format!(
            "reply to request discussion {} for {}/{}",
            params.target.request_id, params.target.owner, params.target.repo
        ),
    )
}

pub fn resolve_request_discussion(
    api: ApiSession<'_>,
    target: RequestTarget<'_>,
    discussion_id: &str,
) -> anyhow::Result<RequestDiscussionMutationResponse> {
    execute(
        api.request(
            reqwest::Method::POST,
            request_discussion_action_path(target, discussion_id, "resolve"),
        ),
        format!(
            "resolve request discussion {} for {}/{}",
            target.request_id, target.owner, target.repo
        ),
    )
}

pub fn reopen_and_reply_to_request_discussion(
    api: ApiSession<'_>,
    params: CreateRequestDiscussionReplyParams<'_>,
) -> anyhow::Result<RequestDiscussionReplyMutationResponse> {
    execute(
        api.request(
            reqwest::Method::POST,
            request_discussion_action_path(params.target, params.discussion_id, "reopen-and-reply"),
        )
        .json(&CreateRequestDiscussionReplyRequest {
            body_markdown: params.body_markdown,
            client_reply_id: params.client_reply_id,
            reply_to_reply_id: None,
            wait_after_reply: false,
        }),
        format!(
            "reopen request discussion {} for {}/{}",
            params.target.request_id, params.target.owner, params.target.repo
        ),
    )
}

fn request_path(target: RequestTarget<'_>) -> String {
    routes::repo_request(target.owner, target.repo, target.request_id)
}

fn request_action_path(target: RequestTarget<'_>, action: &str) -> String {
    routes::repo_request_action(target.owner, target.repo, target.request_id, action)
}

fn request_discussion_action_path(
    target: RequestTarget<'_>,
    discussion_id: &str,
    action: &str,
) -> String {
    routes::repo_request_discussion_action(
        target.owner,
        target.repo,
        target.request_id,
        discussion_id,
        action,
    )
}

pub(super) fn execute<R: DeserializeOwned>(
    request: RequestBuilder,
    context: String,
) -> anyhow::Result<R> {
    let response = request.send().with_context(|| context.clone())?;
    decode_json_response(response, &context)
}

#[cfg(test)]
#[path = "requests_tests.rs"]
mod tests;
