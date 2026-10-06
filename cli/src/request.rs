use crate::api::ApiSession;
use crate::{
    api::{
        CreateRequestDiscussionParams, CreateRequestDiscussionReplyParams, RequestActivityParams,
        RequestTarget, StartRequestParams, add_request_invitee, authorize_request_auto_merge,
        cancel_request_auto_merge, close_request as api_close_request, create_request_discussion,
        create_request_discussion_reply, edit_request_identity, get_request, get_request_activity,
        get_request_auto_merge, leave_request, merge_request, rate_request, remove_request_invitee,
        reopen_and_reply_to_request_discussion, resolve_request_discussion,
        start_request as api_start_request, submit_request as api_submit_request,
    },
    git_repo::{
        GitRepo, current_branch, ensure_clean_working_tree, ensure_git_repo_ready, head_oid,
        request_side_changed_file_paths, run_git_in_repo, try_run_git_in_repo,
        warn_if_dirty_working_tree,
    },
};
use anyhow::{Context, bail};
use scope_api_contract::{ErrorCode, ErrorResponse, RequestDiscussionAnchorInput, ViewId};
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
mod discussion;
use discussion::{DiscussionMutation, discussion_mutation};
mod inspect;
mod local;
pub(crate) use local::{RequestComparison, resolve_request_comparison_ref};
mod outcome;
mod queue;
mod recovery;
mod render;
#[cfg(test)]
mod tests;
mod text;
use crate::display::short_oid;
use actions::*;
pub use args::RequestArgs;
use args::{
    RequestCommand, RequestDiscussionArgs, RequestDiscussionCommand, RequestDiscussionReopenArgs,
    RequestDiscussionReplyArgs, RequestDiscussionResolveArgs, RequestDiscussionStartArgs,
    RequestStartArgs, RequestTargetArgs,
};
use branches::*;
use confirm::require_confirmation;
use local::{
    last_seen_request_head, load_context, load_context_and_request_id,
    maybe_request_id_for_context, push_request_head, refresh_main_projection, remote_main_ref,
    request_id_for_context, store_request_metadata, track_request_branch_ref,
    update_request_remote_ref,
};
use outcome::*;
use queue::{AttentionCommand, change_attention, list_request_queue, queue_outcome};
use render::view_label;
use render::{
    auto_merge_receipt_lines, auto_merge_status_lines, close_receipt, discussion_reopened_receipt,
    discussion_replied_receipt, discussion_resolved_receipt, discussion_started_receipt,
    invitee_added_receipt, invitee_removed_receipt, leave_receipt, repo_access_lines,
    request_activity_lines_for_response, request_detail_lines, request_mutation_receipt_lines,
};
use text::discussion_body;

static CLIENT_MUTATION_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub struct PreparedRequestCommand {
    args: RequestArgs,
    git_repo: Option<GitRepo>,
}

pub fn prepare_request_command(args: RequestArgs) -> anyhow::Result<PreparedRequestCommand> {
    if let Some(repository) = crate::context::explicit_repository() {
        crate::clone::parse_repo_spec(repository)?;
    }
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
            args.remote,
            args.request,
        ),
        RequestCommand::Submit(args) => {
            submit_request_command(git_repo, api, args.target, args.yes)
        }
        RequestCommand::Close(args) => close_request_branch(git_repo, api, args.target, args.yes),
        RequestCommand::Edit(args) => edit_request(git_repo, api, args),
        RequestCommand::Invite(args) => {
            invite_request(git_repo, api, args.target, args.handle, true)
        }
        RequestCommand::Uninvite(args) => {
            invite_request(git_repo, api, args.target, args.handle, false)
        }
        RequestCommand::Leave(args) => leave_invited_request(git_repo, api, args),
        RequestCommand::Merge(args) => merge_request_command(git_repo, api, args),
        RequestCommand::Rate(args) => {
            rate_request_command(git_repo, api, args.target, args.score, args.reason)
        }
        RequestCommand::Discussion(args) => run_request_discussion_command(git_repo, api, args),
        RequestCommand::Show(args) => show_one_request(git_repo, api, args),
        RequestCommand::List(args) => list_request_queue(git_repo, api, args),
        RequestCommand::Claim(target) => {
            change_attention(git_repo, api, target, AttentionCommand::Claim)
        }
        RequestCommand::Release(target) => {
            change_attention(git_repo, api, target, AttentionCommand::Release)
        }
        RequestCommand::Wait(target) => {
            change_attention(git_repo, api, target, AttentionCommand::Wait)
        }
        RequestCommand::Settle(target) => {
            change_attention(git_repo, api, target, AttentionCommand::Settle)
        }
        RequestCommand::Snooze(args) => change_attention(
            git_repo,
            api,
            args.target,
            AttentionCommand::Snooze(args.until),
        ),
        RequestCommand::Restore(target) => {
            change_attention(git_repo, api, target, AttentionCommand::Restore)
        }
        RequestCommand::Checkout(args) => {
            inspect::checkout_request(git_repo.expect("prepared local command"), api, args)
        }
        RequestCommand::Diff(args) => inspect::diff_request(git_repo, api, args),
        RequestCommand::Checks(args) => inspect::request_checks(git_repo, api, args),
        RequestCommand::Status(args) => {
            show_request_status(git_repo, api, args.remote, args.request)
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
        human_lines.extend(request_detail_lines(&detail.request));
        return Ok(RequestCommandOutcome::new(
            "request.status",
            RequestCommandResult::Detail(DetailResult {
                repo: context.repo,
                request: detail.request,
                activity: None,
                auto_merge: None,
            }),
            human_lines,
        ));
    }

    queue_outcome(
        "request.status",
        api,
        context,
        None,
        None,
        queue::QUEUE_SECTION_LIMIT,
    )
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
    let anchor = args
        .revision
        .map(|revision_id| RequestDiscussionAnchorInput {
            revision_id,
            commit_oid: args.commit,
            path: args.path,
        });
    discussion_mutation(
        git_repo,
        api,
        args.target,
        args.content,
        DiscussionMutation::Start(anchor),
    )
}

fn reply_to_request_discussion(
    git_repo: Option<&GitRepo>,
    api: ApiSession<'_>,
    args: RequestDiscussionReplyArgs,
) -> anyhow::Result<RequestCommandOutcome> {
    discussion_mutation(
        git_repo,
        api,
        args.target,
        args.content,
        DiscussionMutation::Reply(args.discussion_id),
    )
}

fn reopen_request_discussion(
    git_repo: Option<&GitRepo>,
    api: ApiSession<'_>,
    args: RequestDiscussionReopenArgs,
) -> anyhow::Result<RequestCommandOutcome> {
    discussion_mutation(
        git_repo,
        api,
        args.target,
        args.content,
        DiscussionMutation::Reopen(args.discussion_id),
    )
}

fn resolve_one_request_discussion(
    git_repo: Option<&GitRepo>,
    api: ApiSession<'_>,
    args: RequestDiscussionResolveArgs,
) -> anyhow::Result<RequestCommandOutcome> {
    let (context, request_id) =
        load_context_and_request_id(git_repo, api, args.target.remote, args.target.request)?;
    let response =
        resolve_request_discussion(api, api_target(&context, &request_id), &args.discussion_id)?;
    let human_lines = vec![discussion_resolved_receipt(&response)];
    Ok(RequestCommandOutcome::new(
        "request.discussion.resolve",
        RequestCommandResult::Discussion(DiscussionResult {
            repo: context.repo,
            request_id,
            discussion: response.discussion,
            attachments: Vec::new(),
        }),
        human_lines,
    ))
}

fn new_client_mutation_id(kind: &str) -> anyhow::Result<String> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before Unix epoch")?
        .as_nanos();
    Ok(format!(
        "client_{kind}_{}_{}_{}",
        std::process::id(),
        nanos,
        CLIENT_MUTATION_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ))
}

fn close_request_branch(
    git_repo: Option<&GitRepo>,
    api: ApiSession<'_>,
    target: RequestTargetArgs,
    yes: bool,
) -> anyhow::Result<RequestCommandOutcome> {
    let (context, request_id, before) = load_exact_request(git_repo, api, target)?;
    let prompt = if before.request.submitted_at_unix.is_none() {
        format!("Permanently delete draft request {}", before.request.name)
    } else {
        format!("Close published request {}", before.request.name)
    };
    require_confirmation(&prompt, yes)?;
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

fn start_view(
    access: &crate::api::RepositoryAccessResponse,
    requested: Option<ViewId>,
) -> anyhow::Result<ViewId> {
    let view = requested.unwrap_or_else(|| access.view.clone());
    if scope_domain::views::Views::builtin()
        .get(&view.clone().into())
        .is_none()
    {
        return Err(crate::error::CliError::usage(format!(
            "Requests target the public or private view until custom views accept requests; pass --view public instead of {}",
            view.as_str()
        ))
        .into());
    }
    let actor = access.actor;
    let author_role = match actor {
        crate::api::RepositoryActor::Public => crate::api::RequestActorRole::Public,
        crate::api::RepositoryActor::Member => crate::api::RequestActorRole::Member,
        crate::api::RepositoryActor::Owner => crate::api::RequestActorRole::Owner,
    };
    scope_domain::requests::validate_start_request_view(author_role.into(), view.clone().into())
        .map_err(|error| crate::error::CliError::usage(error.message))?;
    Ok(view)
}

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
mod view_tests {
    use super::*;
    use crate::api::RepositoryActor;

    #[test]
    fn request_view_defaults_follow_repository_access() {
        for (actor, view) in [
            (RepositoryActor::Owner, ViewId::private()),
            (RepositoryActor::Member, ViewId::private()),
            (RepositoryActor::Public, ViewId::public()),
        ] {
            let access = crate::api::RepositoryAccessResponse {
                actor,
                view: view.clone(),
                can_push: false,
                can_change_file_visibility: false,
                can_manage_members: false,
                can_delete_repo: false,
            };
            assert_eq!(start_view(&access, None).unwrap(), view);
        }
    }
}
