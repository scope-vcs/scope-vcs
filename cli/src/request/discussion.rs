use super::*;
use scope_api_contract::attachments::{RequestAttachmentTargetInput, RequestAttachmentTargetKind};

pub(super) enum DiscussionMutation {
    Start(Option<RequestDiscussionAnchorInput>),
    Reply(String),
    Reopen(String),
}

pub(super) fn discussion_mutation(
    git_repo: Option<&GitRepo>,
    api: ApiSession<'_>,
    target: RequestTargetArgs,
    content: args::RequestDiscussionBodyArgs,
    mutation: DiscussionMutation,
) -> anyhow::Result<RequestCommandOutcome> {
    let attachment_args = content.attachments;
    let body = discussion_body(content.body, content.body_file)?;
    let has_attachments = !attachment_args.paths.is_empty();
    let (context, request_id) =
        load_context_and_request_id(git_repo, api, target.remote, target.request)?;
    let target = api_target(&context, &request_id);
    let (scope, command, kind, discussion_id) = match &mutation {
        DiscussionMutation::Start(_) => (
            "discussion.start",
            "request.discussion.start",
            "discussion",
            None,
        ),
        DiscussionMutation::Reply(id) => (
            "discussion.reply",
            "request.discussion.reply",
            "reply",
            Some(id.as_str()),
        ),
        DiscussionMutation::Reopen(id) => (
            "discussion.reopen",
            "request.discussion.reopen",
            "reply",
            Some(id.as_str()),
        ),
    };
    let uploaded = attachments::upload(
        api,
        target,
        RequestAttachmentTargetInput {
            kind: if discussion_id.is_some() {
                RequestAttachmentTargetKind::Reply
            } else {
                RequestAttachmentTargetKind::Discussion
            },
            discussion_id: discussion_id.map(str::to_owned),
        },
        attachment_args.paths,
    )?;
    let body = text::append_attachment_references(body, uploaded.references);
    let mut scope_parts = vec![
        scope,
        &context.target.owner,
        &context.target.repo,
        &request_id,
    ];
    let anchor_json;
    if let DiscussionMutation::Start(anchor) = &mutation {
        anchor_json = serde_json::to_string(anchor).context("serialize discussion anchor")?;
        scope_parts.extend([body.as_str(), anchor_json.as_str()]);
    } else if let Some(id) = discussion_id {
        scope_parts.extend([id, body.as_str()]);
    }
    let pending_mutation = has_attachments
        .then(|| attachments::begin_mutation(api.base_url, &scope_parts, &format!("cli_{kind}")))
        .transpose()?;
    let client_id = match &pending_mutation {
        Some(mutation) => mutation.client_id.clone(),
        None => new_client_mutation_id(kind)?,
    };
    let (discussion, reply, mut human_lines) = match mutation {
        DiscussionMutation::Start(anchor) => {
            let response = create_request_discussion(
                api,
                CreateRequestDiscussionParams {
                    target,
                    body_markdown: body,
                    client_discussion_id: client_id,
                    anchor,
                },
            )?;
            let lines = discussion_started_receipt(&request_id, &response);
            (response.discussion, None, lines)
        }
        DiscussionMutation::Reply(ref id) | DiscussionMutation::Reopen(ref id) => {
            let params = CreateRequestDiscussionReplyParams {
                target,
                discussion_id: id,
                body_markdown: body,
                client_reply_id: client_id,
            };
            let (response, receipt) = if matches!(mutation, DiscussionMutation::Reopen(_)) {
                let response = reopen_and_reply_to_request_discussion(api, params)?;
                let receipt = discussion_reopened_receipt(&response);
                (response, receipt)
            } else {
                let response = create_request_discussion_reply(api, params)?;
                let receipt = discussion_replied_receipt(&response);
                (response, receipt)
            };
            (response.discussion, Some(response.reply), vec![receipt])
        }
    };
    human_lines.extend(attachment_receipt_lines(&uploaded.attachments));
    let uploaded_attachments = if attachment_args.wait {
        let mut recovery = serde_json::json!({
            "operation": command, "saved": true, "request_id": &request_id, "discussion": &discussion,
        });
        if let Some(reply) = &reply {
            recovery["reply"] =
                serde_json::to_value(reply).context("serialize discussion reply")?;
        }
        attachments::wait_for_processing(api, target, uploaded.attachments, recovery)?
    } else {
        uploaded.attachments
    };
    if let Some(mutation) = &pending_mutation {
        attachments::complete_mutation(mutation, &uploaded.receipt_keys)?;
    }
    let result = match reply {
        Some(reply) => RequestCommandResult::DiscussionReply(DiscussionReplyResult {
            repo: context.repo,
            request_id,
            discussion,
            reply,
            attachments: uploaded_attachments,
        }),
        None => RequestCommandResult::Discussion(DiscussionResult {
            repo: context.repo,
            request_id,
            discussion,
            attachments: uploaded_attachments,
        }),
    };
    Ok(RequestCommandOutcome::new(command, result, human_lines))
}
