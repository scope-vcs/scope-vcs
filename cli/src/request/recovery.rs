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
    let message = format!(
        "request {} exists in {}/{} on local branch '{}'; {} failed: {cause}. {} Retry from this branch with `scope request push --remote {} --request {}`. Do not run request start again.",
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
    );
    CliError::partial(
        message,
        json!({
            "repository": format!("{}/{}", context.target.owner, context.target.repo),
            "request_id": request.id,
            "request_name": request.name,
            "branch": branch,
            "failed_step": failed_step,
            "remote_push_confirmed": push_succeeded,
            "retry_command": retry,
        }),
    )
    .into()
}
