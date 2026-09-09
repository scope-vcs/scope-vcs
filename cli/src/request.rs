use crate::api::ApiSession;
use crate::{
    api::{
        CreateRequestDiscussionParams, CreateRequestDiscussionReplyParams, RequestActivityParams,
        RequestTarget, StartRequestParams, add_request_invitee, close_request as api_close_request,
        create_request_discussion, create_request_discussion_reply, edit_request_identity,
        get_request, get_request_activity, leave_request, list_requests, merge_request,
        rate_request, remove_request_invitee, reopen_and_reply_to_request_discussion,
        resolve_request_discussion, start_request as api_start_request,
        submit_request as api_submit_request,
    },
    git_repo::{
        GitRepo, current_branch, ensure_clean_working_tree, ensure_git_repo_ready, head_oid,
        request_side_changed_file_paths, run_git_in_repo, try_run_git_in_repo,
        warn_if_dirty_working_tree,
    },
};
use anyhow::{Context, bail};
use scope_api_contract::{ErrorCode, ErrorResponse, RequestAudience, RequestDiscussionAnchorInput};
use scope_domain::{policy::ScopePath, repo_control::is_public_request_protected_path};
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

mod actions;
mod args;
mod attachments;
mod branches;
mod confirm;
mod diff;
mod inspect;
mod local;
mod outcome;
mod recovery;
mod remote;
mod render;
#[cfg(test)]
mod tests;
mod text;
use actions::*;
pub use args::RequestArgs;
use args::{
    RequestAudienceArg, RequestCommand, RequestDiscussionArgs, RequestDiscussionCommand,
    RequestDiscussionReopenArgs, RequestDiscussionReplyArgs, RequestDiscussionResolveArgs,
    RequestDiscussionStartArgs, RequestStartArgs, RequestTargetArgs,
};
use branches::*;
use confirm::require_confirmation;
use local::{
    load_context, load_context_and_request_id, maybe_request_id_for_context,
    projection_label_for_audience, push_request_head, refresh_main_projection, remote_main_ref,
    request_id_for_context, store_request_metadata, track_request_branch_ref,
};
use outcome::*;
use render::{
    close_receipt, discussion_reopened_receipt, discussion_replied_receipt,
    discussion_resolved_receipt, discussion_started_receipt, invitee_added_receipt,
    invitee_removed_receipt, leave_receipt, repo_access_lines, request_activity_lines_for_response,
    request_detail_lines_for_response, request_list_line, request_mutation_receipt_lines,
};
use text::{discussion_body, short_oid};

static CLIENT_DISCUSSION_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static CLIENT_REPLY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub struct PreparedRequestCommand {
    args: RequestArgs,
    git_repo: Option<GitRepo>,
}

pub fn prepare_request_command(args: RequestArgs) -> anyhow::Result<PreparedRequestCommand> {
    let local_command = match &args.command {
        RequestCommand::Start(_) => Some(("scope request start", true)),
        RequestCommand::Push(_) => Some(("scope request push", false)),
        RequestCommand::Checkout(_) => Some(("scope request checkout", true)),
        _ => None,
    };
    let git_repo = if let Some((name, clean)) = local_command {
        let repo = ensure_git_repo_ready(name)?;
        if clean {
            ensure_clean_working_tree(&repo, name)?;
        }
        Some(repo)
    } else {
        let repo = crate::context::discover_optional()?;
        if repo.is_none() && crate::context::explicit_repository().is_none() {
            return Err(crate::error::CliError::usage(
                "outside a Git checkout, pass --repo <owner/repo>",
            )
            .into());
        }
        repo
    };
    Ok(PreparedRequestCommand { args, git_repo })
}

pub fn run_request_command(
    command: PreparedRequestCommand,
    api: ApiSession<'_>,
    machine_output: bool,
) -> anyhow::Result<RequestCommandOutcome> {
    let PreparedRequestCommand { args, git_repo } = command;
    let git_repo = git_repo.as_ref();
    match args.command {
        RequestCommand::Start(args) => {
            start_request_branch(git_repo.expect("prepared local command"), api, args)
        }
        RequestCommand::Push(args) => push_request_branch(
            git_repo.expect("prepared local command"),
            api,
            args.target.remote,
            args.target.request,
            machine_output,
        ),
        RequestCommand::Submit(args) => {
            submit_request_command(git_repo, api, args.target, args.yes, machine_output)
        }
        RequestCommand::Close(args) => {
            close_request_branch(git_repo, api, args.target, args.yes, machine_output)
        }
        RequestCommand::Edit(args) => edit_request(git_repo, api, args),
        RequestCommand::Invite(args) => {
            invite_request(git_repo, api, args.target, args.handle, true)
        }
        RequestCommand::Uninvite(args) => {
            invite_request(git_repo, api, args.target, args.handle, false)
        }
        RequestCommand::Leave(args) => leave_invited_request(git_repo, api, args.target),
        RequestCommand::Merge(args) => {
            merge_request_command(git_repo, api, args.target, args.yes, machine_output)
        }
        RequestCommand::Rate(args) => {
            rate_request_command(git_repo, api, args.target, args.score, args.reason)
        }
        RequestCommand::Discussion(args) => run_request_discussion_command(git_repo, api, args),
        RequestCommand::Show(args) => show_one_request(git_repo, api, args.target),
        RequestCommand::List(args) => list_request_status(git_repo, api, args),
        RequestCommand::Checkout(args) => {
            inspect::checkout_request(git_repo.expect("prepared local command"), api, args)
        }
        RequestCommand::Diff(args) => inspect::diff_request(git_repo, api, args),
        RequestCommand::Checks(args) => inspect::request_checks(git_repo, api, args.target),
        RequestCommand::Status(args) => {
            show_request_status(git_repo, api, args.target.remote, args.target.request)
        }
    }
}

fn show_request_status(
    git_repo: Option<&GitRepo>,
    api: ApiSession<'_>,
    remote: Option<String>,
    request_id: Option<String>,
) -> anyhow::Result<RequestCommandOutcome> {
    let context = load_context(git_repo, api, remote.as_deref())?;
    let mut human_lines = repo_access_lines(&context.repo);
    if let Some(request_id) = maybe_request_id_for_context(git_repo, api, &context, request_id)? {
        let detail = get_request(
            api,
            &context.target.owner,
            &context.target.repo,
            &request_id,
        )?;
        human_lines.extend(request_detail_lines_for_response(&detail));
        return Ok(RequestCommandOutcome::new(
            "request.status",
            RequestCommandResult::Detail(DetailResult {
                repo: context.repo,
                request: detail.request,
                activity: None,
            }),
            human_lines,
        ));
    }

    let requests = load_request_list(api, &context)?;
    human_lines.extend(request_list_lines(&requests)?);
    Ok(RequestCommandOutcome::new(
        "request.status",
        RequestCommandResult::List(ListResult {
            repo: context.repo,
            requests,
        }),
        human_lines,
    ))
}

fn run_request_discussion_command(
    git_repo: Option<&GitRepo>,
    api: ApiSession<'_>,
    args: RequestDiscussionArgs,
) -> anyhow::Result<RequestCommandOutcome> {
    match args.command {
        RequestDiscussionCommand::Start(args) => start_request_discussion(git_repo, api, args),
        RequestDiscussionCommand::Reply(args) => reply_to_request_discussion(git_repo, api, args),
        RequestDiscussionCommand::Resolve(args) => {
            resolve_one_request_discussion(git_repo, api, args)
        }
        RequestDiscussionCommand::Reopen(args) => reopen_request_discussion(git_repo, api, args),
    }
}

fn start_request_discussion(
    git_repo: Option<&GitRepo>,
    api: ApiSession<'_>,
    args: RequestDiscussionStartArgs,
) -> anyhow::Result<RequestCommandOutcome> {
    let attachment_args = args.content.attachments;
    let body = discussion_body(args.content.body, args.content.body_file)?;
    let has_attachments = !attachment_args.paths.is_empty();
    let (context, request_id) =
        load_context_and_request_id(git_repo, api, args.target.remote, args.target.request)?;
    let uploaded = attachments::upload(
        api,
        RequestTarget {
            owner: &context.target.owner,
            repo: &context.target.repo,
            request_id: &request_id,
        },
        scope_api_contract::attachments::RequestAttachmentTargetInput {
            kind: scope_api_contract::attachments::RequestAttachmentTargetKind::Discussion,
            discussion_id: None,
        },
        attachment_args.paths,
    )?;
    let body = text::append_attachment_references(body, uploaded.references);
    let anchor = args
        .revision
        .map(|revision_id| RequestDiscussionAnchorInput {
            revision_id,
            commit_oid: args.commit,
            path: args.path,
        });
    let anchor_json = serde_json::to_string(&anchor).context("serialize discussion anchor")?;
    let pending_mutation = has_attachments
        .then(|| {
            attachments::begin_mutation(
                api.base_url,
                &[
                    "discussion.start",
                    &context.target.owner,
                    &context.target.repo,
                    &request_id,
                    &body,
                    &anchor_json,
                ],
                "cli_discussion",
            )
        })
        .transpose()?;
    let client_discussion_id = match &pending_mutation {
        Some(mutation) => mutation.client_id.clone(),
        None => new_client_discussion_id()?,
    };
    let response = create_request_discussion(
        api,
        CreateRequestDiscussionParams {
            target: RequestTarget {
                owner: &context.target.owner,
                repo: &context.target.repo,
                request_id: &request_id,
            },
            body_markdown: body,
            client_discussion_id,
            anchor,
        },
    )?;
    let mut human_lines = discussion_started_receipt(&request_id, &response);
    human_lines.extend(attachment_receipt_lines(&uploaded.attachments));
    let uploaded_attachments = if attachment_args.wait {
        attachments::wait_for_processing(
            api,
            RequestTarget {
                owner: &context.target.owner,
                repo: &context.target.repo,
                request_id: &request_id,
            },
            uploaded.attachments,
            serde_json::json!({
                "operation": "request.discussion.start",
                "saved": true,
                "request_id": &request_id,
                "discussion": &response.discussion,
            }),
        )?
    } else {
        uploaded.attachments
    };
    if let Some(mutation) = &pending_mutation {
        attachments::complete_mutation(mutation, &uploaded.receipt_keys)?;
    }
    if has_attachments {
        return Ok(RequestCommandOutcome::new(
            "request.discussion.start",
            RequestCommandResult::AttachmentDiscussion(AttachmentDiscussionResult {
                repo: context.repo,
                request_id,
                discussion: response.discussion,
                attachments: uploaded_attachments,
            }),
            human_lines,
        ));
    }
    Ok(RequestCommandOutcome::new(
        "request.discussion.start",
        RequestCommandResult::Discussion(DiscussionResult {
            repo: context.repo,
            request_id,
            discussion: response.discussion,
        }),
        human_lines,
    ))
}

fn reply_to_request_discussion(
    git_repo: Option<&GitRepo>,
    api: ApiSession<'_>,
    args: RequestDiscussionReplyArgs,
) -> anyhow::Result<RequestCommandOutcome> {
    let attachment_args = args.content.attachments;
    let body = discussion_body(args.content.body, args.content.body_file)?;
    let has_attachments = !attachment_args.paths.is_empty();
    let (context, request_id) =
        load_context_and_request_id(git_repo, api, args.target.remote, args.target.request)?;
    let uploaded = attachments::upload(
        api,
        RequestTarget {
            owner: &context.target.owner,
            repo: &context.target.repo,
            request_id: &request_id,
        },
        scope_api_contract::attachments::RequestAttachmentTargetInput {
            kind: scope_api_contract::attachments::RequestAttachmentTargetKind::Reply,
            discussion_id: Some(args.discussion_id.clone()),
        },
        attachment_args.paths,
    )?;
    let body = text::append_attachment_references(body, uploaded.references);
    let pending_mutation = has_attachments
        .then(|| {
            attachments::begin_mutation(
                api.base_url,
                &[
                    "discussion.reply",
                    &context.target.owner,
                    &context.target.repo,
                    &request_id,
                    &args.discussion_id,
                    &body,
                ],
                "cli_reply",
            )
        })
        .transpose()?;
    let client_reply_id = match &pending_mutation {
        Some(mutation) => mutation.client_id.clone(),
        None => new_client_reply_id()?,
    };
    let response = create_request_discussion_reply(
        api,
        CreateRequestDiscussionReplyParams {
            target: RequestTarget {
                owner: &context.target.owner,
                repo: &context.target.repo,
                request_id: &request_id,
            },
            discussion_id: &args.discussion_id,
            body_markdown: body,
            client_reply_id,
        },
    )?;
    let mut human_lines = vec![discussion_replied_receipt(&response)];
    human_lines.extend(attachment_receipt_lines(&uploaded.attachments));
    let uploaded_attachments = if attachment_args.wait {
        attachments::wait_for_processing(
            api,
            RequestTarget {
                owner: &context.target.owner,
                repo: &context.target.repo,
                request_id: &request_id,
            },
            uploaded.attachments,
            serde_json::json!({
                "operation": "request.discussion.reply",
                "saved": true,
                "request_id": &request_id,
                "discussion": &response.discussion,
                "reply": &response.reply,
            }),
        )?
    } else {
        uploaded.attachments
    };
    if let Some(mutation) = &pending_mutation {
        attachments::complete_mutation(mutation, &uploaded.receipt_keys)?;
    }
    if has_attachments {
        return Ok(RequestCommandOutcome::new(
            "request.discussion.reply",
            RequestCommandResult::AttachmentDiscussionReply(AttachmentDiscussionReplyResult {
                repo: context.repo,
                request_id,
                discussion: response.discussion,
                reply: response.reply,
                attachments: uploaded_attachments,
            }),
            human_lines,
        ));
    }
    Ok(RequestCommandOutcome::new(
        "request.discussion.reply",
        RequestCommandResult::DiscussionReply(DiscussionReplyResult {
            repo: context.repo,
            request_id,
            discussion: response.discussion,
            reply: response.reply,
        }),
        human_lines,
    ))
}

fn resolve_one_request_discussion(
    git_repo: Option<&GitRepo>,
    api: ApiSession<'_>,
    args: RequestDiscussionResolveArgs,
) -> anyhow::Result<RequestCommandOutcome> {
    let (context, request_id) =
        load_context_and_request_id(git_repo, api, args.target.remote, args.target.request)?;
    let response = resolve_request_discussion(
        api,
        RequestTarget {
            owner: &context.target.owner,
            repo: &context.target.repo,
            request_id: &request_id,
        },
        &args.discussion_id,
    )?;
    let human_lines = vec![discussion_resolved_receipt(&response)];
    Ok(RequestCommandOutcome::new(
        "request.discussion.resolve",
        RequestCommandResult::Discussion(DiscussionResult {
            repo: context.repo,
            request_id,
            discussion: response.discussion,
        }),
        human_lines,
    ))
}

fn reopen_request_discussion(
    git_repo: Option<&GitRepo>,
    api: ApiSession<'_>,
    args: RequestDiscussionReopenArgs,
) -> anyhow::Result<RequestCommandOutcome> {
    let attachment_args = args.content.attachments;
    let body = discussion_body(args.content.body, args.content.body_file)?;
    let has_attachments = !attachment_args.paths.is_empty();
    let (context, request_id) =
        load_context_and_request_id(git_repo, api, args.target.remote, args.target.request)?;
    let uploaded = attachments::upload(
        api,
        RequestTarget {
            owner: &context.target.owner,
            repo: &context.target.repo,
            request_id: &request_id,
        },
        scope_api_contract::attachments::RequestAttachmentTargetInput {
            kind: scope_api_contract::attachments::RequestAttachmentTargetKind::Reply,
            discussion_id: Some(args.discussion_id.clone()),
        },
        attachment_args.paths,
    )?;
    let body = text::append_attachment_references(body, uploaded.references);
    let pending_mutation = has_attachments
        .then(|| {
            attachments::begin_mutation(
                api.base_url,
                &[
                    "discussion.reopen",
                    &context.target.owner,
                    &context.target.repo,
                    &request_id,
                    &args.discussion_id,
                    &body,
                ],
                "cli_reply",
            )
        })
        .transpose()?;
    let client_reply_id = match &pending_mutation {
        Some(mutation) => mutation.client_id.clone(),
        None => new_client_reply_id()?,
    };
    let response = reopen_and_reply_to_request_discussion(
        api,
        CreateRequestDiscussionReplyParams {
            target: RequestTarget {
                owner: &context.target.owner,
                repo: &context.target.repo,
                request_id: &request_id,
            },
            discussion_id: &args.discussion_id,
            body_markdown: body,
            client_reply_id,
        },
    )?;
    let mut human_lines = vec![discussion_reopened_receipt(&response)];
    human_lines.extend(attachment_receipt_lines(&uploaded.attachments));
    let uploaded_attachments = if attachment_args.wait {
        attachments::wait_for_processing(
            api,
            RequestTarget {
                owner: &context.target.owner,
                repo: &context.target.repo,
                request_id: &request_id,
            },
            uploaded.attachments,
            serde_json::json!({
                "operation": "request.discussion.reopen",
                "saved": true,
                "request_id": &request_id,
                "discussion": &response.discussion,
                "reply": &response.reply,
            }),
        )?
    } else {
        uploaded.attachments
    };
    if let Some(mutation) = &pending_mutation {
        attachments::complete_mutation(mutation, &uploaded.receipt_keys)?;
    }
    if has_attachments {
        return Ok(RequestCommandOutcome::new(
            "request.discussion.reopen",
            RequestCommandResult::AttachmentDiscussionReply(AttachmentDiscussionReplyResult {
                repo: context.repo,
                request_id,
                discussion: response.discussion,
                reply: response.reply,
                attachments: uploaded_attachments,
            }),
            human_lines,
        ));
    }
    Ok(RequestCommandOutcome::new(
        "request.discussion.reopen",
        RequestCommandResult::DiscussionReply(DiscussionReplyResult {
            repo: context.repo,
            request_id,
            discussion: response.discussion,
            reply: response.reply,
        }),
        human_lines,
    ))
}

fn new_client_discussion_id() -> anyhow::Result<String> {
    new_client_mutation_id("discussion", &CLIENT_DISCUSSION_SEQUENCE)
}

fn new_client_reply_id() -> anyhow::Result<String> {
    new_client_mutation_id("reply", &CLIENT_REPLY_SEQUENCE)
}

fn new_client_mutation_id(kind: &str, sequence: &AtomicU64) -> anyhow::Result<String> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before Unix epoch")?
        .as_nanos();
    Ok(format!(
        "client_{kind}_{}_{}_{}",
        std::process::id(),
        nanos,
        sequence.fetch_add(1, Ordering::Relaxed)
    ))
}

fn close_request_branch(
    git_repo: Option<&GitRepo>,
    api: ApiSession<'_>,
    target: RequestTargetArgs,
    yes: bool,
    machine_output: bool,
) -> anyhow::Result<RequestCommandOutcome> {
    let (context, request_id, before) = load_exact_request(git_repo, api, target)?;
    let prompt = if before.request.submitted_at_unix.is_none() {
        format!("Permanently delete draft request {}", before.request.name)
    } else {
        format!("Close published request {}", before.request.name)
    };
    require_confirmation(&prompt, yes, !machine_output)?;
    let response = api_close_request(
        api,
        &context.target.owner,
        &context.target.repo,
        &request_id,
    )?;
    let human_line = close_receipt(&request_id, &response);
    Ok(RequestCommandOutcome::new(
        "request.close",
        RequestCommandResult::Close(TargetResponse {
            repo: context.repo,
            request_id,
            response,
        }),
        vec![human_line],
    ))
}

fn start_audience(
    actor: crate::api::RepositoryActor,
    requested: Option<RequestAudienceArg>,
) -> anyhow::Result<crate::api::RequestAudience> {
    use crate::api::RepositoryActor;
    use crate::api::RequestAudience;

    match actor {
        RepositoryActor::Public => match requested.map(Into::into) {
            None | Some(RequestAudience::Public) => Ok(RequestAudience::Public),
            Some(RequestAudience::Private) => {
                bail!("public contributors can only start public requests")
            }
        },
        RepositoryActor::Owner | RepositoryActor::Member => Ok(requested
            .map(Into::into)
            .unwrap_or(RequestAudience::Private)),
    }
}

/// Read the request associated with the current branch without changing local state.
pub fn inspect_current_request(
    git_repo: &GitRepo,
    api: ApiSession<'_>,
    remote: Option<&str>,
) -> anyhow::Result<Option<crate::api::RequestSummaryResponse>> {
    let context = load_context(Some(git_repo), api, remote)?;
    let Some(request_id) = maybe_request_id_for_context(Some(git_repo), api, &context, None)?
    else {
        return Ok(None);
    };
    Ok(Some(
        get_request(
            api,
            &context.target.owner,
            &context.target.repo,
            &request_id,
        )?
        .request,
    ))
}

#[cfg(test)]
mod audience_tests {
    use super::*;
    use crate::api::{RepositoryActor, RequestAudience};

    #[test]
    fn maintainers_default_to_private_requests() {
        for actor in [RepositoryActor::Owner, RepositoryActor::Member] {
            assert_eq!(
                start_audience(actor, None).unwrap(),
                RequestAudience::Private
            );
        }
    }

    #[test]
    fn maintainers_can_explicitly_choose_request_audience() {
        for actor in [RepositoryActor::Owner, RepositoryActor::Member] {
            for (requested, expected) in [
                (RequestAudienceArg::Public, RequestAudience::Public),
                (RequestAudienceArg::Private, RequestAudience::Private),
            ] {
                assert_eq!(start_audience(actor, Some(requested)).unwrap(), expected);
            }
        }
    }

    #[test]
    fn public_contributors_default_to_public_requests() {
        assert_eq!(
            start_audience(RepositoryActor::Public, None).unwrap(),
            RequestAudience::Public
        );
    }

    #[test]
    fn public_contributors_can_only_choose_public_requests() {
        assert_eq!(
            start_audience(RepositoryActor::Public, Some(RequestAudienceArg::Public)).unwrap(),
            RequestAudience::Public
        );
        assert_eq!(
            start_audience(RepositoryActor::Public, Some(RequestAudienceArg::Private))
                .unwrap_err()
                .to_string(),
            "public contributors can only start public requests"
        );
    }
}
