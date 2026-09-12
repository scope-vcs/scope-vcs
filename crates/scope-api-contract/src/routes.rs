// Each route owns its server template, optional Rust builder, and optional web name.
// The exported group order is the generated TypeScript declaration order.
macro_rules! define_route {
    ($constant:ident, $path:literal $(, $builder:ident($($arg:ident: $ty:ty),* $(,)?))?) => {
        pub const $constant: &str = $path;
        $(pub fn $builder($($arg: $ty),*) -> String {
            format!($path, $($arg = path_segment(&$arg.to_string())),*)
        })?
    };
}

macro_rules! routes {
    (
        exported { $($constant:ident = $path:literal => $web:literal
            $(, $builder:ident($($arg:ident: $ty:ty),* $(,)?))?;)* }
        internal { $($internal:ident = $internal_path:literal
            $(, $internal_builder:ident($($internal_arg:ident: $internal_ty:ty),* $(,)?))?;)* }
    ) => {
        $(define_route!($constant, $path $(, $builder($($arg: $ty),*))?);)*
        $(define_route!($internal, $internal_path $(, $internal_builder($($internal_arg: $internal_ty),*))?);)*
        pub const WEB_ROUTE_TEMPLATES: &[(&str, &str)] = &[$(($web, $constant)),*];
    };
}

routes! {
    exported {
        ACCOUNT_SESSION = "/v1/session" => "accountSession";
        CLI_DEVICE_LOGIN_COMPLETE = "/v1/cli/device-login/{user_code}/complete"
            => "cliDeviceLoginComplete";
        CLI_BROWSER_LOGIN_COMPLETE = "/v1/cli/browser-login/{request_id}/complete"
            => "cliBrowserLoginComplete";
        CLI_EXCHANGE_GRANTS = "/v1/cli/exchange-grants" => "cliExchangeGrants";
        CLI_SESSIONS = "/v1/cli/sessions" => "cliSessions";
        CLI_SESSION_BY_ID = "/v1/cli/sessions/{session_id}" => "cliSessionById";
        REPOS = "/v1/repos" => "repos";
        OWNER_REPOSITORIES = "/v1/users/{handle}/repos"
            => "ownerRepositories",
            owner_repositories(handle: &str);
        REPO = "/v1/repos/{owner}/{repo}" => "repo", repo(owner: &str, repo: &str);
        REPO_CONFIG = "/v1/repos/{owner}/{repo}/config"
            => "repoConfig",
            repo_config(owner: &str, repo: &str);
        REPO_METADATA = "/v1/repos/{owner}/{repo}/metadata" => "repoMetadata";
        REPO_DEPENDENCIES = "/v1/repos/{owner}/{repo}/dependencies"
            => "repoDependencies",
            repo_dependencies(owner: &str, repo: &str);
        REPO_RUN_WORKFLOWS = "/v1/repos/{owner}/{repo}/run-workflows"
            => "repoRunWorkflows",
            repo_run_workflows(owner: &str, repo: &str);
        REPO_RUNS = "/v1/repos/{owner}/{repo}/runs" => "repoRuns", repo_runs(owner: &str, repo: &str);
        REPO_RUN_DETAIL = "/v1/repos/{owner}/{repo}/runs/{run_id}/detail"
            => "repoRunDetail",
            repo_run_detail(owner: &str, repo: &str, run_id: &str);
        REPO_RUN_STEP_LOGS = "/v1/repos/{owner}/{repo}/runs/{run_id}/attempts/{attempt_id}/steps/{step_index}/logs"
            => "repoRunStepLogs",
            repo_run_step_logs(owner: &str, repo: &str, run_id: &str, attempt_id: &str, step_index: u32);
        REPO_RUN_CANCEL = "/v1/repos/{owner}/{repo}/runs/{run_id}/cancel"
            => "repoRunCancel",
            repo_run_cancel(owner: &str, repo: &str, run_id: &str);
        REPO_RUN_RETRY = "/v1/repos/{owner}/{repo}/runs/{run_id}/retry"
            => "repoRunRetry",
            repo_run_retry(owner: &str, repo: &str, run_id: &str);
        REPO_PUSH_INTENTS = "/v1/repos/{owner}/{repo}/push-intents"
            => "repoPushIntents",
            repo_push_intents(owner: &str, repo: &str);
        REPO_REQUESTS = "/v1/repos/{owner}/{repo}/requests"
            => "repoRequests",
            repo_requests(owner: &str, repo: &str);
        REPO_REQUEST_QUEUE = "/v1/repos/{owner}/{repo}/requests/queue" => "repoRequestQueue";
        REPO_REQUEST_ATTENTION = "/v1/repos/{owner}/{repo}/requests/{request_id}/attention"
            => "repoRequestAttention",
            repo_request_attention(owner: &str, repo: &str, request_id: &str);
        REPO_REQUEST = "/v1/repos/{owner}/{repo}/requests/{request_id}"
            => "repoRequest",
            repo_request(owner: &str, repo: &str, request_id: &str);
        REPO_REQUEST_SUBMIT = "/v1/repos/{owner}/{repo}/requests/{request_id}/submit"
            => "repoRequestSubmit",
            repo_request_submit(owner: &str, repo: &str, request_id: &str);
        REPO_REQUEST_MERGE = "/v1/repos/{owner}/{repo}/requests/{request_id}/merge"
            => "repoRequestMerge",
            repo_request_merge(owner: &str, repo: &str, request_id: &str);
        REPO_REQUEST_RATINGS = "/v1/repos/{owner}/{repo}/requests/{request_id}/ratings"
            => "repoRequestRatings",
            repo_request_ratings(owner: &str, repo: &str, request_id: &str);
        REPO_REQUEST_INVITEES = "/v1/repos/{owner}/{repo}/requests/{request_id}/invitees"
            => "repoRequestInvitees",
            repo_request_invitees(owner: &str, repo: &str, request_id: &str);
        REPO_REQUEST_INVITEES_ME = "/v1/repos/{owner}/{repo}/requests/{request_id}/invitees/me"
            => "repoRequestInviteesMe",
            repo_request_invitees_me(owner: &str, repo: &str, request_id: &str);
        REPO_REQUEST_ATTACHMENTS = "/v1/repos/{owner}/{repo}/requests/{request_id}/attachments"
            => "repoRequestAttachments",
            repo_request_attachments(owner: &str, repo: &str, request_id: &str);
        REPO_REQUEST_ATTACHMENT_LIMITS = "/v1/repos/{owner}/{repo}/requests/{request_id}/attachments/limits"
            => "repoRequestAttachmentLimits",
            repo_request_attachment_limits(owner: &str, repo: &str, request_id: &str);
        REPO_REQUEST_ATTACHMENT_PREPARE = "/v1/repos/{owner}/{repo}/requests/{request_id}/attachments/prepare"
            => "repoRequestAttachmentPrepare",
            repo_request_attachment_prepare(owner: &str, repo: &str, request_id: &str);
        REPO_REQUEST_ATTACHMENT = "/v1/repos/{owner}/{repo}/requests/{request_id}/attachments/{attachment_id}"
            => "repoRequestAttachment",
            repo_request_attachment(owner: &str, repo: &str, request_id: &str, attachment_id: &str);
        REPO_REQUEST_ATTACHMENT_FINISH = "/v1/repos/{owner}/{repo}/requests/{request_id}/attachments/{attachment_id}/finish"
            => "repoRequestAttachmentFinish",
            repo_request_attachment_finish(
                owner: &str,
                repo: &str,
                request_id: &str,
                attachment_id: &str,
            );
        REPO_REQUEST_ATTACHMENT_RETRY = "/v1/repos/{owner}/{repo}/requests/{request_id}/attachments/{attachment_id}/retry"
            => "repoRequestAttachmentRetry",
            repo_request_attachment_retry(
                owner: &str,
                repo: &str,
                request_id: &str,
                attachment_id: &str,
            );
        REPO_REQUEST_ATTACHMENT_MEDIA_GRANT = "/v1/repos/{owner}/{repo}/requests/{request_id}/attachments/{attachment_id}/media-grant"
            => "repoRequestAttachmentMediaGrant",
            repo_request_attachment_media_grant(
                owner: &str,
                repo: &str,
                request_id: &str,
                attachment_id: &str,
            );
        REPO_FILES = "/v1/repos/{owner}/{repo}/files" => "repoFiles";
        REPO_FILE_CONTENT = "/v1/repos/{owner}/{repo}/files/content" => "repoFileContent";
        REPO_REQUEST_REVISIONS = "/v1/repos/{owner}/{repo}/requests/{request_id}/changes"
            => "repoRequestRevisions",
            repo_request_revisions(owner: &str, repo: &str, request_id: &str);
        REPO_REQUEST_REVISION_COMMIT_FILE_DIFF = "/v1/repos/{owner}/{repo}/requests/{request_id}/changes/{revision_id}/commits/{commit_oid}/file-diff"
            => "repoRequestRevisionCommitFileDiff",
            repo_request_revision_commit_file_diff(
                owner: &str,
                repo: &str,
                request_id: &str,
                revision_id: &str,
                commit_oid: &str,
            );
        REPO_REQUEST_DISCUSSIONS = "/v1/repos/{owner}/{repo}/requests/{request_id}/timeline"
            => "repoRequestDiscussions";
        REPO_REQUEST_DISCUSSION_CHANGES = "/v1/repos/{owner}/{repo}/requests/{request_id}/timeline/changes"
            => "repoRequestDiscussionChanges";
        REPO_REQUEST_DISCUSSION_REPLIES = "/v1/repos/{owner}/{repo}/requests/{request_id}/threads/{discussion_id}/replies"
            => "repoRequestDiscussionReplies";
        REPO_REQUEST_DISCUSSION_RESOLVE = "/v1/repos/{owner}/{repo}/requests/{request_id}/threads/{discussion_id}/resolve"
            => "repoRequestDiscussionResolve";
        REPO_REQUEST_DISCUSSION_REOPEN = "/v1/repos/{owner}/{repo}/requests/{request_id}/threads/{discussion_id}/reopen"
            => "repoRequestDiscussionReopen";
        REPO_REQUEST_DISCUSSION_REOPEN_AND_REPLY = "/v1/repos/{owner}/{repo}/requests/{request_id}/threads/{discussion_id}/reopen-and-reply"
            => "repoRequestDiscussionReopenAndReply";
        REPO_REQUEST_DISCUSSION_READ = "/v1/repos/{owner}/{repo}/requests/{request_id}/threads/{discussion_id}/read"
            => "repoRequestDiscussionRead";
        REPO_REQUEST_ACTIVITY = "/v1/repos/{owner}/{repo}/requests/{request_id}/activity"
            => "repoRequestActivity";
        REPO_EVENTS = "/v1/repos/{owner}/{repo}/events" => "repoEvents";
        REPO_HISTORY = "/v1/repos/{owner}/{repo}/history" => "repoHistory";
        REPO_HISTORY_ENTRY = "/v1/repos/{owner}/{repo}/history/{entry_id}" => "repoHistoryEntry";
        REPO_HISTORY_ENTRY_FILE_DIFF = "/v1/repos/{owner}/{repo}/history/{entry_id}/file-diff"
            => "repoHistoryEntryFileDiff";
        REPO_MEMBERS = "/v1/repos/{owner}/{repo}/members" => "repoMembers";
        REPO_INVITES = "/v1/repos/{owner}/{repo}/invites" => "repoInvites";
        REPO_INVITE = "/v1/repos/{owner}/{repo}/invites/{invite_id}" => "repoInvite";
        REPO_MEMBER = "/v1/repos/{owner}/{repo}/members/{member_user_id}" => "repoMember";
        REPOSITORY_INVITE = "/v1/repository-invites/{token}" => "repositoryInvite";
        REPOSITORY_INVITE_ACCEPT = "/v1/repository-invites/{token}/accept" => "repositoryInviteAccept";
        REPO_PROJECTION_PREVIEW = "/v1/repos/{owner}/{repo}/projection-preview"
            => "repoProjectionPreview";
        GIT_REPO = "/git/{mode}/{org}/{repo}" => "gitRepo", git_repo(mode: &str, org: &str, repo: &str);
    }
    internal {
        HEALTH = "/healthz";
        READINESS = "/readyz";
        ADMIN_CLEANUP = "/v1/admin/cleanup";
        ADMIN_CLEANUP_DRAIN = "/v1/admin/cleanup/drain";
        CLI_BROWSER_LOGIN = "/v1/cli/browser-login";
        CLI_BROWSER_LOGIN_EXCHANGE = "/v1/cli/browser-login/{request_id}/exchange",
            cli_browser_login_exchange(request_id: &str);
        CLI_DEVICE_LOGIN = "/v1/cli/device-login";
        CLI_DEVICE_LOGIN_POLL = "/v1/cli/device-login/{device_code}/poll",
            cli_device_login_poll(device_code: &str);
        CLI_EXCHANGE_GRANTS_EXCHANGE = "/v1/cli/exchange-grants/exchange";
        CLI_SESSION = "/v1/cli/session";
        ATTEMPT_CLAIM = "/v1/runtime-protocol/attempts/{attempt_id}/claim",
            attempt_claim(attempt_id: &str);
        ATTEMPT_HEARTBEAT = "/v1/runtime-protocol/attempts/{attempt_id}/heartbeat",
            attempt_heartbeat(attempt_id: &str);
        ATTEMPT_CACHE_PREPARATIONS = "/v1/runtime-protocol/attempts/{attempt_id}/cache-observations/preparations",
            attempt_cache_preparations(attempt_id: &str);
        ATTEMPT_CACHE_FINALIZATIONS = "/v1/runtime-protocol/attempts/{attempt_id}/cache-observations/finalizations",
            attempt_cache_finalizations(attempt_id: &str);
        ATTEMPT_RECOVERY_STATUS = "/v1/runtime-protocol/attempts/{attempt_id}/recovery-status",
            attempt_recovery_status(attempt_id: &str);
        ATTEMPT_SOURCE = "/v1/runtime-protocol/attempts/{attempt_id}/source",
            attempt_source(attempt_id: &str);
        ATTEMPT_LOGS = "/v1/runtime-protocol/attempts/{attempt_id}/logs", attempt_logs(attempt_id: &str);
        ATTEMPT_COMPLETE = "/v1/runtime-protocol/attempts/{attempt_id}/complete",
            attempt_complete(attempt_id: &str);
        ATTEMPT_ABANDON = "/v1/runtime-protocol/attempts/{attempt_id}/abandon",
            attempt_abandon(attempt_id: &str);
        ATTEMPT_STEP_START = "/v1/runtime-protocol/attempts/{attempt_id}/steps/{step_index}/start",
            attempt_step_start(attempt_id: &str, step_index: u32);
        ATTEMPT_STEP_COMPLETE = "/v1/runtime-protocol/attempts/{attempt_id}/steps/{step_index}/complete",
            attempt_step_complete(attempt_id: &str, step_index: u32);
        REPO_RUN_RESOLVE = "/v1/repos/{owner}/{repo}/runs/resolve",
            repo_run_resolve(owner: &str, repo: &str);
        REPO_RUN = "/v1/repos/{owner}/{repo}/runs/{run_id}",
            repo_run(owner: &str, repo: &str, run_id: &str);
        REPO_RUN_EVENTS = "/v1/repos/{owner}/{repo}/runs/{run_id}/events",
            repo_run_events(owner: &str, repo: &str, run_id: &str);
        REPO_PUSH_TRIGGER_EVALUATION = "/v1/repos/{owner}/{repo}/push-trigger-evaluations/{head_oid}",
            repo_push_trigger_evaluation(owner: &str, repo: &str, head_oid: &str);
        MEDIA_UPLOAD_PART = "/v1/uploads/{upload_id}/parts/{part_number}",
            media_upload_part(upload_id: &str, part_number: u32);
        MEDIA_ATTACHMENT_ORIGINAL = "/v1/attachments/{attachment_id}/original",
            media_attachment_original(attachment_id: &str);
        MEDIA_ATTACHMENT_DERIVATIVE = "/v1/attachments/{attachment_id}/derivatives/{derivative_id}",
            media_attachment_derivative(attachment_id: &str, derivative_id: &str);
        GIT_INFO_REFS = "/git/{mode}/{org}/{repo}/info/refs";
        GIT_RECEIVE_PACK = "/git/{mode}/{org}/{repo}/git-receive-pack";
        GIT_UPLOAD_PACK = "/git/{mode}/{org}/{repo}/git-upload-pack";
        DEV_BENCH_CLI_SESSION = "/v1/dev/bench/cli-session";
        DEV_CLI_SESSION = "/v1/dev/cli-session/{handle}";
    }
}

pub fn repo_request_action(owner: &str, repo: &str, request_id: &str, action: &str) -> String {
    format!(
        "{}/{}",
        repo_request(owner, repo, request_id),
        path_segment(action)
    )
}

pub fn repo_request_discussion(
    owner: &str,
    repo: &str,
    request_id: &str,
    discussion_id: &str,
) -> String {
    format!(
        "{}/threads/{}",
        repo_request(owner, repo, request_id),
        path_segment(discussion_id)
    )
}

pub fn repo_request_discussion_action(
    owner: &str,
    repo: &str,
    request_id: &str,
    discussion_id: &str,
    action: &str,
) -> String {
    format!(
        "{}/{}",
        repo_request_discussion(owner, repo, request_id, discussion_id),
        path_segment(action)
    )
}

pub fn path_segment(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')'
            )
        {
            encoded.push(char::from(byte));
        } else {
            use std::fmt::Write as _;
            write!(encoded, "%{byte:02X}").expect("writing to a String cannot fail");
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dynamic_routes_encode_each_path_segment() {
        let routes = [
            (
                attempt_cache_preparations("attempt/with space"),
                "/v1/runtime-protocol/attempts/attempt%2Fwith%20space/cache-observations/preparations",
            ),
            (
                attempt_cache_finalizations("attempt/with space"),
                "/v1/runtime-protocol/attempts/attempt%2Fwith%20space/cache-observations/finalizations",
            ),
            (
                attempt_step_complete("attempt/with space", 3),
                "/v1/runtime-protocol/attempts/attempt%2Fwith%20space/steps/3/complete",
            ),
            (
                repo_request("an owner", "r/name", "request?#1"),
                "/v1/repos/an%20owner/r%2Fname/requests/request%3F%231",
            ),
            (
                repo_request_submit("an owner", "r/name", "request?#1"),
                "/v1/repos/an%20owner/r%2Fname/requests/request%3F%231/submit",
            ),
            (
                repo_request_merge("an owner", "r/name", "request?#1"),
                "/v1/repos/an%20owner/r%2Fname/requests/request%3F%231/merge",
            ),
            (
                repo_request_discussion_action(
                    "an owner",
                    "r/name",
                    "request?#1",
                    "thread/#1",
                    "reopen-and-reply",
                ),
                "/v1/repos/an%20owner/r%2Fname/requests/request%3F%231/threads/thread%2F%231/reopen-and-reply",
            ),
            (
                repo_request_revision_commit_file_diff(
                    "owner",
                    "repo",
                    "req/one",
                    "rev/old",
                    "commit?#1",
                ),
                "/v1/repos/owner/repo/requests/req%2Fone/changes/rev%2Fold/commits/commit%3F%231/file-diff",
            ),
            (
                repo_request_attachment_media_grant(
                    "an owner",
                    "r/name",
                    "request?#1",
                    "attachment/#1",
                ),
                "/v1/repos/an%20owner/r%2Fname/requests/request%3F%231/attachments/attachment%2F%231/media-grant",
            ),
            (
                media_upload_part("upload/with space", 4),
                "/v1/uploads/upload%2Fwith%20space/parts/4",
            ),
            (
                media_attachment_derivative("attachment/#1", "preview #1"),
                "/v1/attachments/attachment%2F%231/derivatives/preview%20%231",
            ),
            (
                cli_device_login_poll("code/with space"),
                "/v1/cli/device-login/code%2Fwith%20space/poll",
            ),
            (
                repo_run_step_logs("an owner", "r/name", "run?#1", "attempt/#1", 3),
                "/v1/repos/an%20owner/r%2Fname/runs/run%3F%231/attempts/attempt%2F%231/steps/3/logs",
            ),
            (
                git_repo("permissioned", "an owner", "r/name"),
                "/git/permissioned/an%20owner/r%2Fname",
            ),
        ];
        for (actual, expected) in routes {
            assert_eq!(actual, expected);
        }
    }
}
