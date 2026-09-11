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
    pub owner: &'a str,
    pub repo: &'a str,
    pub name: String,
    pub title: Option<String>,
    pub audience: RequestAudience,
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
    let mut request = api.request(
        reqwest::Method::GET,
        scope_api_contract::routes::repo_requests(owner, repo),
    );
    if let Some(cursor) = cursor {
        request = request.query(&[("cursor", cursor)]);
    }
    execute_repo_request(request, owner, repo, "list requests")
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
    execute_request(
        api.request(reqwest::Method::GET, request_path(target)),
        target,
        "load request",
    )
}

pub fn request_revisions(
    api: ApiSession<'_>,
    target: RequestTarget<'_>,
    revision: Option<&str>,
    commit: Option<&str>,
) -> anyhow::Result<RequestRevisionListResponse> {
    let mut request = api.request(
        reqwest::Method::GET,
        routes::repo_request_revisions(target.owner, target.repo, target.request_id),
    );
    if let Some(revision) = revision {
        request = request.query(&[("revision", revision)]);
    }
    if let Some(commit) = commit {
        request = request.query(&[("commit", commit)]);
    }
    execute_request(request, target, "inspect request revisions")
}

pub struct RequestFileDiffParams<'a> {
    pub target: RequestTarget<'a>,
    pub revision: &'a str,
    pub commit: &'a str,
    pub path: &'a str,
}

pub fn request_file_diff(
    api: ApiSession<'_>,
    params: RequestFileDiffParams<'_>,
) -> anyhow::Result<ReviewFileDiffResponse> {
    execute_request(
        api.request(
            reqwest::Method::GET,
            routes::repo_request_revision_commit_file_diff(
                params.target.owner,
                params.target.repo,
                params.target.request_id,
                params.revision,
                params.commit,
            ),
        )
        .query(&[("path", params.path)]),
        params.target,
        "inspect request file diff",
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
    execute_request(
        api.request(reqwest::Method::DELETE, request_path(target)),
        target,
        "close request",
    )
}

pub fn start_request(
    api: ApiSession<'_>,
    params: StartRequestParams<'_>,
) -> anyhow::Result<RequestMutationResponse> {
    let owner = params.owner;
    let repo = params.repo;
    execute_repo_request(
        api.request(
            reqwest::Method::POST,
            scope_api_contract::routes::repo_requests(owner, repo),
        )
        .json(&StartRequestRequest {
            name: params.name,
            title: params.title,
            audience: params.audience,
        }),
        owner,
        repo,
        "start request",
    )
}

pub fn submit_request(
    api: ApiSession<'_>,
    target: RequestTarget<'_>,
) -> anyhow::Result<RequestMutationResponse> {
    execute_request(
        api.request(reqwest::Method::POST, request_action_path(target, "submit"))
            .json(&SubmitRequestRequest {}),
        target,
        "submit request",
    )
}

pub fn merge_request(
    api: ApiSession<'_>,
    target: RequestTarget<'_>,
) -> anyhow::Result<RequestMutationResponse> {
    execute_request(
        api.request(reqwest::Method::POST, request_action_path(target, "merge")),
        target,
        "merge request",
    )
}

pub fn rate_request(
    api: ApiSession<'_>,
    target: RequestTarget<'_>,
    score: u8,
    reason: String,
) -> anyhow::Result<RequestRatingResponse> {
    execute_request(
        api.request(
            reqwest::Method::POST,
            request_action_path(target, "ratings"),
        )
        .json(&CreateRequestRatingRequest { score, reason }),
        target,
        "rate request participant",
    )
}

pub fn edit_request_identity(
    api: ApiSession<'_>,
    target: RequestTarget<'_>,
    title: Option<String>,
    description_markdown: Option<String>,
    expected_description_markdown: Option<String>,
) -> anyhow::Result<RequestMutationResponse> {
    execute_request(
        api.request(reqwest::Method::PATCH, request_path(target))
            .json(&EditRequestIdentityRequest {
                title,
                description_markdown,
                expected_description_markdown,
            }),
        target,
        "edit request identity",
    )
}

pub fn add_request_invitee(
    api: ApiSession<'_>,
    target: RequestTarget<'_>,
    handle: String,
) -> anyhow::Result<RequestInviteeMutationResponse> {
    execute_request(
        api.request(
            reqwest::Method::PUT,
            request_action_path(target, "invitees"),
        )
        .json(&AddRequestInviteeRequest { handle }),
        target,
        "invite request collaborator",
    )
}

pub fn remove_request_invitee(
    api: ApiSession<'_>,
    target: RequestTarget<'_>,
    handle: String,
) -> anyhow::Result<RequestInviteeMutationResponse> {
    execute_request(
        api.request(
            reqwest::Method::DELETE,
            request_action_path(target, "invitees"),
        )
        .json(&RemoveRequestInviteeRequest { handle }),
        target,
        "remove request invitee",
    )
}

pub fn leave_request(
    api: ApiSession<'_>,
    target: RequestTarget<'_>,
) -> anyhow::Result<LeaveRequestResponse> {
    execute_request(
        api.request(
            reqwest::Method::DELETE,
            scope_api_contract::routes::repo_request_invitees_me(
                target.owner,
                target.repo,
                target.request_id,
            ),
        ),
        target,
        "leave request",
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
    execute_request(request, params.target, "load request activity")
}

pub fn create_request_discussion(
    api: ApiSession<'_>,
    params: CreateRequestDiscussionParams<'_>,
) -> anyhow::Result<RequestDiscussionMutationResponse> {
    let target = params.target;
    execute_request(
        api.request(
            reqwest::Method::POST,
            request_action_path(target, "timeline"),
        )
        .json(&CreateRequestDiscussionRequest {
            body_markdown: params.body_markdown,
            client_discussion_id: params.client_discussion_id,
            anchor: params.anchor,
        }),
        target,
        "create request discussion",
    )
}

pub fn create_request_discussion_reply(
    api: ApiSession<'_>,
    params: CreateRequestDiscussionReplyParams<'_>,
) -> anyhow::Result<RequestDiscussionReplyMutationResponse> {
    execute_request(
        api.request(
            reqwest::Method::POST,
            request_discussion_action_path(params.target, params.discussion_id, "replies"),
        )
        .json(&CreateRequestDiscussionReplyRequest {
            body_markdown: params.body_markdown,
            client_reply_id: params.client_reply_id,
            reply_to_reply_id: None,
        }),
        params.target,
        "reply to request discussion",
    )
}

pub fn resolve_request_discussion(
    api: ApiSession<'_>,
    target: RequestTarget<'_>,
    discussion_id: &str,
) -> anyhow::Result<RequestDiscussionMutationResponse> {
    execute_request(
        api.request(
            reqwest::Method::POST,
            request_discussion_action_path(target, discussion_id, "resolve"),
        ),
        target,
        "resolve request discussion",
    )
}

pub fn reopen_and_reply_to_request_discussion(
    api: ApiSession<'_>,
    params: CreateRequestDiscussionReplyParams<'_>,
) -> anyhow::Result<RequestDiscussionReplyMutationResponse> {
    execute_request(
        api.request(
            reqwest::Method::POST,
            request_discussion_action_path(params.target, params.discussion_id, "reopen-and-reply"),
        )
        .json(&ReopenAndReplyRequest {
            body_markdown: params.body_markdown,
            client_reply_id: params.client_reply_id,
            reply_to_reply_id: None,
        }),
        params.target,
        "reopen request discussion",
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

fn execute_repo_request<R: DeserializeOwned>(
    request: RequestBuilder,
    owner: &str,
    repo: &str,
    action: &str,
) -> anyhow::Result<R> {
    let context = format!("{action} for {owner}/{repo}");
    let response = request.send().with_context(|| context.clone())?;
    decode_json_response(response, &context)
}

pub(super) fn execute_request<R: DeserializeOwned>(
    request: RequestBuilder,
    target: RequestTarget<'_>,
    action: &str,
) -> anyhow::Result<R> {
    let context = format!(
        "{action} {} for {}/{}",
        target.request_id, target.owner, target.repo
    );
    let response = request.send().with_context(|| context.clone())?;
    decode_json_response(response, &context)
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::{StatusCode, blocking::Client};
    use std::{io::Write, net::TcpListener, thread};

    #[test]
    fn list_requests_sends_the_opaque_cursor_as_a_query_parameter() {
        let (api_url, server) = serve_once(StatusCode::OK, r#"{"requests":[],"next_cursor":null}"#);

        let response = list_requests(
            ApiSession::new(&Client::new(), &api_url, "token"),
            "owner",
            "repo",
            Some("next page/+"),
        )
        .unwrap();

        assert!(response.requests.is_empty());
        assert!(response.next_cursor.is_none());
        let request = server.join().unwrap();
        assert!(
            request
                .lines()
                .next()
                .unwrap()
                .ends_with("?cursor=next+page%2F%2B HTTP/1.1"),
            "{request}"
        );
    }

    #[test]
    fn request_errors_surface_authoritative_safe_messages() {
        let (api_url, server) = serve_once(
            StatusCode::CONFLICT,
            r#"{"code":"conflict","message":"request cannot be submitted\u001b[31m","retryable":false}"#,
        );

        let error = submit_request(ApiSession::new(&Client::new(), &api_url, "token"), target())
            .unwrap_err()
            .to_string();

        assert_eq!(error, "request cannot be submitted [31m");
        let request = server.join().unwrap();
        assert!(request.starts_with("POST /v1/repos/owner/repo/requests/req_one/submit "));
        assert!(request.contains("\r\n\r\n{}"), "{request}");
    }

    #[test]
    fn diagnostic_errors_surface_only_the_safe_message_and_reference() {
        let (api_url, server) = serve_once(
            StatusCode::INTERNAL_SERVER_ERROR,
            r#"{"code":"internal","message":"Scope hit an internal error.","error_reference":"err_0123456789abcdef0123456789abcdef\u001b[31m","retryable":false}"#,
        );

        let error = get_request(
            ApiSession::new(&Client::new(), &api_url, "token"),
            "owner",
            "repo",
            "req_one",
        )
        .unwrap_err()
        .to_string();

        assert_eq!(
            error,
            "Scope hit an internal error.\nReference: err_0123456789abcdef0123456789abcdef [31m"
        );
        server.join().unwrap();
    }

    #[test]
    fn request_errors_surface_the_cli_upgrade_instruction() {
        let (api_url, server) = serve_once(
            StatusCode::UPGRADE_REQUIRED,
            r#"{"code":"cli_upgrade_required","message":"installed Scope CLI protocol 0; this API supports protocol 1","instruction":"Upgrade with `curl -fsSL https://scope-cli-production.up.railway.app/install.sh | sh`, then retry.","fields":{"installed_protocol":0,"supported_protocol":1},"retryable":false}"#,
        );

        let error = submit_request(ApiSession::new(&Client::new(), &api_url, "token"), target())
            .unwrap_err()
            .to_string();

        assert!(error.contains("installed Scope CLI protocol 0"), "{error}");
        assert!(error.contains(CLI_INSTALL_COMMAND), "{error}");
        server.join().unwrap();
    }

    #[test]
    fn submit_posts_an_empty_payload() {
        let (api_url, server) = serve_once(
            StatusCode::CONFLICT,
            r#"{"code":"conflict","message":"fixture stop","retryable":false}"#,
        );

        submit_request(ApiSession::new(&Client::new(), &api_url, "token"), target()).unwrap_err();

        let request = server.join().unwrap();
        assert!(request.contains("\r\n\r\n{}"), "{request}");
    }

    #[test]
    fn request_not_found_uses_the_authoritative_contract_message() {
        let (api_url, server) = serve_once(
            StatusCode::NOT_FOUND,
            r#"{"code":"not_found","message":"request req_one not found in owner/repo","retryable":false}"#,
        );

        let error = get_request(
            ApiSession::new(&Client::new(), &api_url, "token"),
            "owner",
            "repo",
            "req_one",
        )
        .unwrap_err()
        .to_string();

        assert_eq!(error, "request req_one not found in owner/repo");
        server.join().unwrap();
    }

    #[test]
    fn malformed_error_bodies_use_a_scoped_status_fallback() {
        let (api_url, server) = serve_once(StatusCode::SERVICE_UNAVAILABLE, "upstream exploded");

        let error = merge_request(ApiSession::new(&Client::new(), &api_url, "token"), target())
            .unwrap_err()
            .to_string();

        assert_eq!(
            error,
            "Scope is temporarily unavailable while trying to merge request req_one for owner/repo"
        );
        server.join().unwrap();
    }

    #[test]
    fn invite_and_activity_wrappers_use_contract_methods_queries_and_payloads() {
        let (api_url, invite_server) = serve_once(
            StatusCode::CONFLICT,
            r#"{"code":"conflict","message":"fixture stop","retryable":false}"#,
        );
        add_request_invitee(
            ApiSession::new(&Client::new(), &api_url, "token"),
            target(),
            "Exact-Handle".to_string(),
        )
        .unwrap_err();
        let invite_request = invite_server.join().unwrap();
        assert!(
            invite_request
                .starts_with("PUT /v1/repos/owner/repo/requests/req_one/invitees HTTP/1.1")
        );
        assert!(
            invite_request.contains(r#"{"handle":"Exact-Handle"}"#),
            "{invite_request}"
        );

        let (api_url, activity_server) =
            serve_once(StatusCode::OK, r#"{"events":[],"through_position":7}"#);
        let page = get_request_activity(
            ApiSession::new(&Client::new(), &api_url, "token"),
            RequestActivityParams {
                target: target(),
                after: Some(4),
                latest: true,
                limit: Some(25),
            },
        )
        .unwrap();
        assert!(page.events.is_empty());
        assert_eq!(page.through_position, 7);
        let activity_request = activity_server.join().unwrap();
        let request_line = activity_request.lines().next().unwrap();
        assert!(
            request_line.starts_with("GET /v1/repos/owner/repo/requests/req_one/activity?"),
            "{request_line}"
        );
        for query in ["after=4", "latest=true", "limit=25"] {
            assert!(request_line.contains(query), "{request_line}");
        }
    }

    #[test]
    fn discussion_wrappers_use_explicit_thread_routes_and_payloads() {
        let stopped = r#"{"code":"conflict","message":"fixture stop","retryable":false}"#;

        let (api_url, start_server) = serve_once(StatusCode::CONFLICT, stopped);
        create_request_discussion(
            ApiSession::new(&Client::new(), &api_url, "token"),
            CreateRequestDiscussionParams {
                target: target(),
                body_markdown: "Question\\n".to_string(),
                client_discussion_id: "client-discussion".to_string(),
                anchor: Some(RequestDiscussionAnchorInput {
                    revision_id: "rev_one".to_string(),
                    commit_oid: Some("0123456789abcdef".to_string()),
                    path: Some("src/lib.rs".to_string()),
                }),
            },
        )
        .unwrap_err();
        let request = start_server.join().unwrap();
        assert!(
            request.starts_with("POST /v1/repos/owner/repo/requests/req_one/timeline HTTP/1.1")
        );
        assert!(
            request.contains(r#""body_markdown":"Question\\n""#),
            "{request}"
        );
        assert!(request.contains(r#""revision_id":"rev_one""#), "{request}");
        assert!(request.contains(r#""path":"src/lib.rs""#), "{request}");

        let (api_url, reply_server) = serve_once(StatusCode::CONFLICT, stopped);
        create_request_discussion_reply(
            ApiSession::new(&Client::new(), &api_url, "token"),
            CreateRequestDiscussionReplyParams {
                target: target(),
                discussion_id: "dsc /one",
                body_markdown: "Answer".to_string(),
                client_reply_id: "client-reply".to_string(),
            },
        )
        .unwrap_err();
        let request = reply_server.join().unwrap();
        assert!(
            request.starts_with(
                "POST /v1/repos/owner/repo/requests/req_one/threads/dsc%20%2Fone/replies HTTP/1.1"
            ),
            "{request}"
        );
        assert!(request.contains(r#""client_reply_id":"client-reply""#));
        assert!(request.contains(r#""reply_to_reply_id":null"#));

        let (api_url, resolve_server) = serve_once(StatusCode::CONFLICT, stopped);
        resolve_request_discussion(
            ApiSession::new(&Client::new(), &api_url, "token"),
            target(),
            "dsc_one",
        )
        .unwrap_err();
        let request = resolve_server.join().unwrap();
        assert!(request.starts_with(
            "POST /v1/repos/owner/repo/requests/req_one/threads/dsc_one/resolve HTTP/1.1"
        ));

        let (api_url, reopen_server) = serve_once(StatusCode::CONFLICT, stopped);
        reopen_and_reply_to_request_discussion(
            ApiSession::new(&Client::new(), &api_url, "token"),
            CreateRequestDiscussionReplyParams {
                target: target(),
                discussion_id: "dsc_one",
                body_markdown: "New evidence".to_string(),
                client_reply_id: "client-reopen".to_string(),
            },
        )
        .unwrap_err();
        let request = reopen_server.join().unwrap();
        assert!(request.starts_with(
            "POST /v1/repos/owner/repo/requests/req_one/threads/dsc_one/reopen-and-reply HTTP/1.1"
        ));
        assert!(request.contains(r#""body_markdown":"New evidence""#));
    }

    fn target() -> RequestTarget<'static> {
        RequestTarget {
            owner: "owner",
            repo: "repo",
            request_id: "req_one",
        }
    }

    fn serve_once(status: StatusCode, body: &'static str) -> (String, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = crate::test_support::read_http_request(&mut stream).unwrap();
            assert!(
                request
                    .lines()
                    .any(|line| line.eq_ignore_ascii_case("authorization: Bearer token")),
                "authenticated endpoint omitted its session token: {request}"
            );
            write!(
                stream,
                "HTTP/1.1 {} {}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                status.as_u16(),
                status.canonical_reason().unwrap_or("Unknown"),
                body.len(),
            )
            .unwrap();
            request
        });
        (format!("http://{address}"), server)
    }
}
