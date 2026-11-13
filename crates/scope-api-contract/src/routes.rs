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
        pub const ROUTE_TEMPLATES: &[&str] = &[$($constant,)* $($internal,)*];
    };
}

routes! {
    exported {
        ACCOUNT = "/v1/account" => "account";
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
        REPO_GITHUB = "/v1/repos/{owner}/{repo}/github" => "repoGitHub";
        REPO_GITHUB_AUTHORIZE = "/v1/repos/{owner}/{repo}/github/authorize" => "repoGitHubAuthorize";
        REPO_GITHUB_REQUIRED_CHECKS = "/v1/repos/{owner}/{repo}/github/required-checks"
            => "repoGitHubRequiredChecks";
        REPO_GITHUB_REVIEW_BOTS = "/v1/repos/{owner}/{repo}/github/review-bots"
            => "repoGitHubReviewBots";
        REPO_GITHUB_REVIEW_BOT_PULL_REQUESTS = "/v1/repos/{owner}/{repo}/github/review-bot-pull-requests"
            => "repoGitHubReviewBotPullRequests";
        REPO_GITHUB_REVIEW_REQUEST_COMMENT = "/v1/repos/{owner}/{repo}/github/review-request-comment"
            => "repoGitHubReviewRequestComment";
        REPO_GITHUB_PUBLIC_CONFIRMATION = "/v1/repos/{owner}/{repo}/github/public-confirmation"
            => "repoGitHubPublicConfirmation";
        REPO_GITHUB_SETUP_CHECK = "/v1/repos/{owner}/{repo}/github/setup-check"
            => "repoGitHubSetupCheck";
        REPO_GITHUB_MAIN_PUBLICATION = "/v1/repos/{owner}/{repo}/github/main-publication"
            => "repoGitHubMainPublication";
        REPO_GITHUB_WORKFLOW_RUNS = "/v1/repos/{owner}/{repo}/github/workflow-runs"
            => "repoGitHubWorkflowRuns";
        REPO_GITHUB_WORKFLOW_NAMES = "/v1/repos/{owner}/{repo}/github/workflow-names"
            => "repoGitHubWorkflowNames";
        REPO_GITHUB_WORKFLOW_RUN = "/v1/repos/{owner}/{repo}/github/workflow-runs/{run_id}"
            => "repoGitHubWorkflowRun";
        REPO_GITHUB_WORKFLOW_JOB_LOG = "/v1/repos/{owner}/{repo}/github/workflow-runs/{run_id}/jobs/{job_id}/log"
            => "repoGitHubWorkflowJobLog";
        REPO_GITHUB_RUN_IMPORT = "/v1/repos/{owner}/{repo}/github/run-import"
            => "repoGitHubRunImport";
        GITHUB_SETUP = "/v1/github/setup" => "githubSetup";
        REPO_DEPENDENCIES = "/v1/repos/{owner}/{repo}/dependencies"
            => "repoDependencies",
            repo_dependencies(owner: &str, repo: &str);
        REPO_PUSH_INTENTS = "/v1/repos/{owner}/{repo}/push-intents"
            => "repoPushIntents",
            repo_push_intents(owner: &str, repo: &str);
        REPO_REQUESTS = "/v1/repos/{owner}/{repo}/requests"
            => "repoRequests",
            repo_requests(owner: &str, repo: &str);
        REPO_REQUEST_QUEUE = "/v1/repos/{owner}/{repo}/requests/queue"
            => "repoRequestQueue",
            repo_request_queue(owner: &str, repo: &str);
        REPO_REQUEST_ATTENTION = "/v1/repos/{owner}/{repo}/requests/{request_id}/attention"
            => "repoRequestAttention",
            repo_request_attention(owner: &str, repo: &str, request_id: &str);
        REPO_REQUEST = "/v1/repos/{owner}/{repo}/requests/{request_id}"
            => "repoRequest",
            repo_request(owner: &str, repo: &str, request_id: &str);
        REPO_REQUEST_CHILDREN = "/v1/repos/{owner}/{repo}/requests/{request_id}/children" => "repoRequestChildren";
        REPO_REQUEST_STATE = "/v1/repos/{owner}/{repo}/requests/{request_id}/state"
            => "repoRequestState",
            repo_request_state(owner: &str, repo: &str, request_id: &str);
        REPO_REQUEST_SUBMIT = "/v1/repos/{owner}/{repo}/requests/{request_id}/submit"
            => "repoRequestSubmit",
            repo_request_submit(owner: &str, repo: &str, request_id: &str);
        REPO_REQUEST_MERGE = "/v1/repos/{owner}/{repo}/requests/{request_id}/merge"
            => "repoRequestMerge",
            repo_request_merge(owner: &str, repo: &str, request_id: &str);
        REPO_REQUEST_AUTO_MERGE = "/v1/repos/{owner}/{repo}/requests/{request_id}/auto-merge"
            => "repoRequestAutoMerge",
            repo_request_auto_merge(owner: &str, repo: &str, request_id: &str);
        REPO_REQUEST_CHECKS = "/v1/repos/{owner}/{repo}/requests/{request_id}/checks"
            => "repoRequestChecks",
            repo_request_checks(owner: &str, repo: &str, request_id: &str);
        REPO_REQUEST_CHECKS_APPROVE = "/v1/repos/{owner}/{repo}/requests/{request_id}/checks/approve"
            => "repoRequestChecksApprove",
            repo_request_checks_approve(owner: &str, repo: &str, request_id: &str);
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
        REPO_REQUEST_REVISION_CONTEXT = "/v1/repos/{owner}/{repo}/requests/{request_id}/revision-context"
            => "repoRequestRevisionContext",
            repo_request_revision_context(owner: &str, repo: &str, request_id: &str);
        REPO_REQUEST_REVISION_FILE_DIFF = "/v1/repos/{owner}/{repo}/requests/{request_id}/changes/{revision_id}/file-diff"
            => "repoRequestRevisionFileDiff",
            repo_request_revision_file_diff(owner: &str, repo: &str, request_id: &str, revision_id: &str);
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
        REPO_HISTORY = "/v1/repos/{owner}/{repo}/history" => "repoHistory", repo_history(owner: &str, repo: &str);
        REPO_HISTORY_ENTRY = "/v1/repos/{owner}/{repo}/history/{entry_id}" => "repoHistoryEntry";
        REPO_HISTORY_ENTRY_FILE_DIFF = "/v1/repos/{owner}/{repo}/history/{entry_id}/file-diff"
            => "repoHistoryEntryFileDiff";
        REPO_MEMBERS = "/v1/repos/{owner}/{repo}/members" => "repoMembers";
        REPO_INVITES = "/v1/repos/{owner}/{repo}/invites" => "repoInvites";
        REPO_INVITE = "/v1/repos/{owner}/{repo}/invites/{invite_id}" => "repoInvite";
        REPO_INVITE_LINKS = "/v1/repos/{owner}/{repo}/invites/{invite_id}/links" => "repoInviteLinks";
        REPO_INVITE_EMAILS = "/v1/repos/{owner}/{repo}/invites/{invite_id}/emails" => "repoInviteEmails";
        REPO_MEMBER = "/v1/repos/{owner}/{repo}/members/{member_user_id}" => "repoMember";
        REPOSITORY_INVITE = "/v1/repository-invites/{token}" => "repositoryInvite";
        REPOSITORY_INVITE_ACCEPT = "/v1/repository-invites/{token}/accept" => "repositoryInviteAccept";
        REPO_PROJECTION_PREVIEW = "/v1/repos/{owner}/{repo}/projection-preview"
            => "repoProjectionPreview";
        GIT_REPO = "/git/{view}/{org}/{repo}" => "gitRepo", git_repo(view: &str, org: &str, repo: &str);
    }
    internal {
        HEALTH = "/healthz";
        READINESS = "/readyz";
        MCP = "/mcp";
        MCP_PROTECTED_RESOURCE_METADATA = "/.well-known/oauth-protected-resource/mcp";
        CLI_BROWSER_LOGIN = "/v1/cli/browser-login";
        CLI_BROWSER_LOGIN_EXCHANGE = "/v1/cli/browser-login/{request_id}/exchange",
            cli_browser_login_exchange(request_id: &str);
        CLI_DEVICE_LOGIN = "/v1/cli/device-login";
        CLI_DEVICE_LOGIN_POLL = "/v1/cli/device-login/{device_code}/poll",
            cli_device_login_poll(device_code: &str);
        CLI_EXCHANGE_GRANTS_EXCHANGE = "/v1/cli/exchange-grants/exchange";
        CLI_SESSION = "/v1/cli/session";
        GITHUB_WEBHOOKS = "/v1/github/webhooks";
        MEDIA_UPLOAD_PART = "/v1/uploads/{upload_id}/parts/{part_number}",
            media_upload_part(upload_id: &str, part_number: u32);
        MEDIA_ATTACHMENT_ORIGINAL = "/v1/attachments/{attachment_id}/original",
            media_attachment_original(attachment_id: &str);
        MEDIA_ATTACHMENT_DERIVATIVE = "/v1/attachments/{attachment_id}/derivatives/{derivative_id}",
            media_attachment_derivative(attachment_id: &str, derivative_id: &str);
        GIT_INFO_REFS = "/git/{view}/{org}/{repo}/info/refs";
        GIT_RECEIVE_PACK = "/git/{view}/{org}/{repo}/git-receive-pack";
        GIT_UPLOAD_PACK = "/git/{view}/{org}/{repo}/git-upload-pack";
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
            #[expect(clippy::expect_used)]
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
                repo_request_auto_merge("an owner", "r/name", "request?#1"),
                "/v1/repos/an%20owner/r%2Fname/requests/request%3F%231/auto-merge",
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
                git_repo("agent", "an owner", "r/name"),
                "/git/agent/an%20owner/r%2Fname",
            ),
        ];
        for (actual, expected) in routes {
            assert_eq!(actual, expected);
        }
    }
}
