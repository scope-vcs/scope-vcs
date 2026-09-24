use super::local::RequestContext;
use crate::{api::RequestSummaryResponse, error::CliError};
use serde_json::json;

pub(super) fn request_partial(
    context: &RequestContext,
    request: &RequestSummaryResponse,
    branch: &str,
    failed_step: &str,
    push_succeeded: bool,
    cause: anyhow::Error,
) -> anyhow::Error {
    let retry = vec![
        "scope",
        "request",
        "push",
        "--remote",
        &context.target.remote,
        "--request",
        &request.id,
    ];
    let needs_checkout = matches!(failed_step, "save_local_metadata" | "configure_tracking");
    let binding_recovery = if needs_checkout {
        format!(
            " After the push succeeds, run `scope request checkout --remote {} --request {} --branch {branch}` to restore the local branch association.",
            context.target.remote, request.id
        )
    } else {
        String::new()
    };
    let message = format!(
        "request {} exists in {}/{} on local branch '{}'; {} failed: {cause}. {} Retry from this branch with `scope request push --remote {} --request {}`.{} Do not run request start again.",
        request.id,
        context.target.owner,
        context.target.repo,
        branch,
        failed_step,
        if push_succeeded {
            "The request head was pushed successfully."
        } else {
            "The request head has not been confirmed pushed."
        },
        context.target.remote,
        request.id,
        binding_recovery,
    );
    let mut receipt = json!({
        "repository": format!("{}/{}", context.target.owner, context.target.repo),
        "request_id": request.id,
        "request_name": request.name,
        "branch": branch,
        "failed_step": failed_step,
        "remote_push_confirmed": push_succeeded,
        "retry_command": retry,
    });
    if needs_checkout {
        receipt["follow_up_command"] = json!([
            "scope",
            "request",
            "checkout",
            "--remote",
            context.target.remote,
            "--request",
            request.id,
            "--branch",
            branch,
        ]);
    }
    CliError::partial(message, receipt).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git_transport::ScopeRemote;

    #[test]
    fn partial_tracking_failure_reports_push_then_checkout() {
        let target = ScopeRemote::parse(
            "https://scope.example",
            "scope",
            "https://scope.example/git/permissioned/owner/repo",
        )
        .unwrap();
        let context = RequestContext {
            target,
            repo: serde_json::from_value(json!({
                "id":"repo_one", "owner_handle":"owner", "name":"repo",
                "git_remote_url":"https://scope.example/git/public/owner/repo",
                "lifecycle_state":"Ready", "change_version":1, "open_request_count":1,
                "access":{"actor":"Public", "can_read_private_files":false,
                    "can_push":false, "can_change_file_visibility":false,
                    "can_manage_members":false, "can_delete_repo":false}
            }))
            .unwrap(),
        };
        let head = "a".repeat(40);
        let request: RequestSummaryResponse = serde_json::from_value(json!({
            "id":"req_one", "name":"fix-one", "title":"Fix one",
            "description_markdown":"", "author_user_id":"usr_one",
            "author_role":"Public", "audience":"Public",
            "base_main_oid":head, "head_oid":head, "state":"Draft",
            "activity_version":0, "submitted_at_unix":null, "closed_at_unix":null,
            "closed_by_user_id":null, "merged_at_unix":null,
            "merged_by_user_id":null, "merged_head_oid":null, "merged_main_oid":null,
            "created_at_unix":1, "updated_at_unix":1, "invitees":[],
            "permissions":{"can_view_activity":false, "can_open_discussion":false,
                "can_reply_to_discussion":false, "can_wait_after_reply":false,
                "can_edit_identity":false, "can_pull_branch":true,
                "can_push_branch":true, "can_submit":false,
                "can_manage_invitees":false, "can_leave_request":false,
                "can_close":false, "can_merge":false},
            "mergeability":{"status":"Draft", "current_main_oid":head,
                "request_head_oid":head, "reason":null}
        }))
        .unwrap();

        let error = request_partial(
            &context,
            &request,
            "fix-one",
            "configure_tracking",
            true,
            anyhow::anyhow!("Git config is locked"),
        );
        let envelope = crate::error::json_response(&error);
        let receipt = envelope.recovery.unwrap();
        assert!(envelope.error.message.contains("request checkout"));
        assert_eq!(receipt["failed_step"], "configure_tracking");
        assert_eq!(receipt["remote_push_confirmed"], true);
        assert_eq!(
            receipt["retry_command"],
            json!([
                "scope",
                "request",
                "push",
                "--remote",
                "scope",
                "--request",
                "req_one"
            ])
        );
        assert_eq!(
            receipt["follow_up_command"],
            json!([
                "scope",
                "request",
                "checkout",
                "--remote",
                "scope",
                "--request",
                "req_one",
                "--branch",
                "fix-one"
            ])
        );

        let error = request_partial(
            &context,
            &request,
            "fix-one",
            "push_request_head",
            false,
            anyhow::anyhow!("push failed"),
        );
        let receipt = crate::error::json_response(&error).recovery.unwrap();
        assert!(receipt.get("follow_up_command").is_none());
    }
}
