use super::request_access::visible_request;
use crate::persistence_ids::generate_prefixed_id;
use crate::{
    error::ApiError, persistence::unix_now, product_analytics::ProductEvent,
    repo_access::find_read_access, state::AppState,
};
use scope_domain::{
    account::UserAccount,
    repository::access::RepositoryAccessContext,
    requests::{
        MarkRequestDiscussionReadInput, Request, RequestDiscussionReply, request_actor_role,
    },
};
pub(crate) use scope_postgres::db::DiscussionTransition;
use scope_postgres::db::{
    CreateRequestDiscussionCommand, CreateRequestDiscussionReplyCommand,
    ReopenAndReplyToRequestDiscussionCommand, RequestDiscussionReadModel,
    RequestDiscussionReplyReadModel, TransitionRequestDiscussionCommand,
};
use std::collections::{BTreeMap, BTreeSet};

mod anchor;
#[cfg(test)]
mod tests;

pub(crate) struct DiscussionAnchorInput {
    pub(crate) revision_id: String,
    pub(crate) commit_oid: Option<String>,
    pub(crate) path: Option<String>,
}

pub(crate) struct CreateDiscussionCommand {
    pub(crate) owner: String,
    pub(crate) repo_name: String,
    pub(crate) request_id: String,
    pub(crate) actor_user_id: String,
    pub(crate) client_discussion_id: String,
    pub(crate) body_markdown: String,
    pub(crate) anchor: Option<DiscussionAnchorInput>,
}

pub(crate) struct ReplyCommand {
    pub(crate) reopen_discussion: bool,
    pub(crate) owner: String,
    pub(crate) repo_name: String,
    pub(crate) request_id: String,
    pub(crate) discussion_id: String,
    pub(crate) actor_user_id: String,
    pub(crate) client_reply_id: String,
    pub(crate) body_markdown: String,
    pub(crate) reply_to_reply_id: Option<String>,
}

pub(crate) struct TransitionDiscussionCommand {
    pub(crate) owner: String,
    pub(crate) repo_name: String,
    pub(crate) request_id: String,
    pub(crate) discussion_id: String,
    pub(crate) actor_user_id: String,
    pub(crate) transition: DiscussionTransition,
}

pub(crate) struct MarkDiscussionReadCommand {
    pub(crate) owner: String,
    pub(crate) repo_name: String,
    pub(crate) request_id: String,
    pub(crate) discussion_id: String,
    pub(crate) actor_user_id: String,
    pub(crate) through_position: u64,
}

pub(crate) struct DiscussionMutationResult {
    pub(crate) discussion: RequestDiscussionReadModel,
    pub(crate) users: BTreeMap<String, UserAccount>,
    pub(crate) visible_anchor_commits: BTreeSet<(String, String)>,
}

pub(crate) struct ReplyMutationResult {
    pub(crate) discussion: DiscussionMutationResult,
    pub(crate) reply: RequestDiscussionReplyReadModel,
    pub(crate) reply_users: BTreeMap<String, UserAccount>,
}

pub(super) struct MutationContext {
    pub(super) repo: RepositoryAccessContext,
    pub(super) request: Request,
}

pub(crate) async fn create_discussion(
    state: &AppState,
    command: CreateDiscussionCommand,
) -> Result<DiscussionMutationResult, ApiError> {
    let context = mutation_context(
        state,
        &command.owner,
        &command.repo_name,
        &command.request_id,
        &command.actor_user_id,
    )
    .await?;
    let anchor = match command.anchor {
        Some(anchor) => Some(anchor::validate(state, &context, anchor).await?),
        None => None,
    };
    let mutation = state
        .metadata
        .requests()
        .create_request_discussion(CreateRequestDiscussionCommand {
            request_id: context.request.id.clone(),
            id: generate_prefixed_id("discussion_")?,
            actor_user_id: command.actor_user_id.clone(),
            client_discussion_id: command.client_discussion_id,
            body_markdown: command.body_markdown,
            anchor,
            now_unix: unix_now()?,
        })
        .await?;
    if mutation.created {
        state
            .product_analytics
            .capture(ProductEvent::discussion_created(
                &command.actor_user_id,
                context.request.audience,
                request_actor_role(context.repo.access),
                mutation.discussion.anchor.is_some(),
            ));
    }
    let discussion_id = mutation.discussion.id.clone();
    let through_position = mutation.discussion.last_activity_position;
    publish_timeline_change(state, &context, discussion_id.clone(), through_position).await;
    load_discussion_result(state, &context, &discussion_id, &command.actor_user_id).await
}

pub(crate) async fn reply(
    state: &AppState,
    command: ReplyCommand,
) -> Result<ReplyMutationResult, ApiError> {
    let context = mutation_context(
        state,
        &command.owner,
        &command.repo_name,
        &command.request_id,
        &command.actor_user_id,
    )
    .await?;
    let requests = state.metadata.requests();
    let mutation = if command.reopen_discussion {
        requests
            .reopen_and_reply_to_request_discussion(ReopenAndReplyToRequestDiscussionCommand {
                request_id: context.request.id.clone(),
                discussion_id: command.discussion_id,
                reply_id: generate_prefixed_id("discussion_reply_")?,
                actor_user_id: command.actor_user_id.clone(),
                event_id: generate_prefixed_id("event_request_discussion_reopened_")?,
                client_reply_id: command.client_reply_id,
                body_markdown: command.body_markdown,
                reply_to_reply_id: command.reply_to_reply_id,
                now_unix: unix_now()?,
            })
            .await?
    } else {
        requests
            .create_request_discussion_reply(CreateRequestDiscussionReplyCommand {
                request_id: context.request.id.clone(),
                discussion_id: command.discussion_id,
                id: generate_prefixed_id("discussion_reply_")?,
                actor_user_id: command.actor_user_id.clone(),
                client_reply_id: command.client_reply_id,
                body_markdown: command.body_markdown,
                reply_to_reply_id: command.reply_to_reply_id,
                now_unix: unix_now()?,
            })
            .await?
    };
    reply_mutation_result(
        state,
        &context,
        mutation.discussion.id,
        mutation.reply,
        &command.actor_user_id,
    )
    .await
}

pub(crate) async fn transition_discussion(
    state: &AppState,
    command: TransitionDiscussionCommand,
) -> Result<DiscussionMutationResult, ApiError> {
    let context = mutation_context(
        state,
        &command.owner,
        &command.repo_name,
        &command.request_id,
        &command.actor_user_id,
    )
    .await?;
    let event_prefix = match command.transition {
        DiscussionTransition::Resolve => "event_request_discussion_resolved_",
        DiscussionTransition::Reopen => "event_request_discussion_reopened_",
    };
    let discussion = state
        .metadata
        .requests()
        .transition_request_discussion(TransitionRequestDiscussionCommand {
            request_id: context.request.id.clone(),
            discussion_id: command.discussion_id.clone(),
            actor_user_id: command.actor_user_id.clone(),
            event_id: generate_prefixed_id(event_prefix)?,
            now_unix: unix_now()?,
            transition: command.transition,
        })
        .await?;
    if matches!(command.transition, DiscussionTransition::Resolve) {
        state
            .product_analytics
            .capture(ProductEvent::discussion_resolved(
                &command.actor_user_id,
                context.request.audience,
                request_actor_role(context.repo.access),
            ));
    }
    let through_position = discussion.last_activity_position;
    publish_timeline_change(
        state,
        &context,
        command.discussion_id.clone(),
        through_position,
    )
    .await;
    load_discussion_result(
        state,
        &context,
        &command.discussion_id,
        &command.actor_user_id,
    )
    .await
}

pub(crate) async fn mark_read(
    state: &AppState,
    command: MarkDiscussionReadCommand,
) -> Result<u64, ApiError> {
    let context = mutation_context(
        state,
        &command.owner,
        &command.repo_name,
        &command.request_id,
        &command.actor_user_id,
    )
    .await?;
    ensure_discussion_in_request(state, &context.request.id, &command.discussion_id).await?;
    let read_state = state
        .metadata
        .requests()
        .mark_request_discussion_read(MarkRequestDiscussionReadInput {
            discussion_id: command.discussion_id,
            user_id: command.actor_user_id,
            through_position: command.through_position,
            now_unix: unix_now()?,
        })
        .await?;
    Ok(read_state.read_through_position)
}

async fn mutation_context(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    request_id: &str,
    actor_user_id: &str,
) -> Result<MutationContext, ApiError> {
    let repo = find_read_access(state, owner, repo_name, Some(actor_user_id)).await?;
    let access = repo.access;
    let request = visible_request(
        state,
        &repo.record.id,
        access,
        Some(actor_user_id),
        request_id,
    )
    .await?;
    Ok(MutationContext { repo, request })
}

async fn reply_mutation_result(
    state: &AppState,
    context: &MutationContext,
    discussion_id: String,
    reply: RequestDiscussionReply,
    actor_user_id: &str,
) -> Result<ReplyMutationResult, ApiError> {
    publish_timeline_change(state, context, discussion_id.clone(), reply.position).await;
    let discussion = load_discussion_result(state, context, &discussion_id, actor_user_id).await?;
    let (reply, reply_users) = state
        .metadata
        .requests()
        .request_discussion_reply_read_model(reply)
        .await?;
    Ok(ReplyMutationResult {
        discussion,
        reply,
        reply_users,
    })
}

async fn load_discussion_result(
    state: &AppState,
    context: &MutationContext,
    discussion_id: &str,
    viewer_user_id: &str,
) -> Result<DiscussionMutationResult, ApiError> {
    let (discussion, users) = state
        .metadata
        .requests()
        .request_discussion(&context.request.id, discussion_id, Some(viewer_user_id))
        .await?
        .ok_or_else(|| ApiError::not_found("request discussion not found"))?;
    let visible_anchor_commits =
        anchor::visible_commits(state, context, discussion.discussion.anchor.as_ref()).await;
    Ok(DiscussionMutationResult {
        discussion,
        users,
        visible_anchor_commits,
    })
}

async fn publish_timeline_change(
    state: &AppState,
    context: &MutationContext,
    discussion_id: String,
    through_position: u64,
) {
    state
        .publish_request_timeline_change(
            &context.repo.incarnation(),
            context.request.id.clone(),
            discussion_id,
            through_position,
            context.request.audience,
        )
        .await;
}

async fn ensure_discussion_in_request(
    state: &AppState,
    request_id: &str,
    discussion_id: &str,
) -> Result<(), ApiError> {
    state
        .metadata
        .requests()
        .request_discussion(request_id, discussion_id, None)
        .await?
        .ok_or_else(|| ApiError::not_found("request discussion not found"))?;
    Ok(())
}
