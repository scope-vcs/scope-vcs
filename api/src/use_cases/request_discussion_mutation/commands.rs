use scope_domain::requests::{
    CreateRequestDiscussionInput, CreateRequestDiscussionMutation,
    CreateRequestDiscussionReplyInput, CreateRequestDiscussionReplyMutation,
    MarkRequestDiscussionReadInput, ReopenAndReplyToRequestDiscussionInput, RequestDiscussion,
    RequestDiscussionReadState, RequestDiscussionTransitionInput, create_request_discussion,
    create_request_discussion_reply, ensure_request_discussion_transition_allowed,
    mark_request_discussion_read, reopen_and_reply_to_request_discussion,
    reopen_request_discussion, resolve_request_discussion,
};
use scope_postgres::{
    db::{RequestStore, RequestTransaction},
    error::PostgresError,
};

#[derive(Clone, Debug)]
pub(crate) struct CreateRequestDiscussionReplyCommand {
    pub(crate) request_id: String,
    pub(crate) discussion_id: String,
    pub(crate) id: String,
    pub(crate) actor_user_id: String,
    pub(crate) client_reply_id: String,
    pub(crate) body_markdown: String,
    pub(crate) reply_to_reply_id: Option<String>,
    pub(crate) wait_after_reply: bool,
    pub(crate) now_unix: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct ReopenAndReplyToRequestDiscussionCommand {
    pub(crate) request_id: String,
    pub(crate) discussion_id: String,
    pub(crate) reply_id: String,
    pub(crate) actor_user_id: String,
    pub(crate) event_id: String,
    pub(crate) client_reply_id: String,
    pub(crate) body_markdown: String,
    pub(crate) reply_to_reply_id: Option<String>,
    pub(crate) wait_after_reply: bool,
    pub(crate) now_unix: u64,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum DiscussionTransition {
    Resolve,
    Reopen,
}

#[derive(Clone, Debug)]
pub(crate) struct TransitionRequestDiscussionCommand {
    pub(crate) request_id: String,
    pub(crate) discussion_id: String,
    pub(crate) actor_user_id: String,
    pub(crate) event_id: String,
    pub(crate) now_unix: u64,
    pub(crate) transition: DiscussionTransition,
}

pub(crate) async fn persist_discussion(
    store: &RequestStore,
    command: CreateRequestDiscussionInput,
) -> Result<CreateRequestDiscussionMutation, PostgresError> {
    let tx = store
        .begin_request(&command.request_id, &command.actor_user_id)
        .await?;
    let now_unix = command.now_unix;
    if let Some(discussion) = tx
        .discussion_by_client_id(&command.client_discussion_id)
        .await?
    {
        if !tx
            .actor()
            .policy(tx.request(), &command.actor_user_id)
            .discussion_visible
        {
            return Err(PostgresError::not_found("request discussion not found"));
        }
        let read_state = match tx.discussion_read_state(&discussion.id).await? {
            Some(state) => state,
            None => {
                read_through(
                    &tx,
                    &discussion,
                    &command.actor_user_id,
                    discussion.opened_position,
                    now_unix,
                )
                .await?
            }
        };
        let request = tx.request().clone();
        tx.commit().await?;
        return Ok(CreateRequestDiscussionMutation {
            created: false,
            request,
            discussion,
            read_state,
        });
    }
    let discussion_id_exists = tx.discussion_id_exists(&command.id).await?;
    let mutation = create_request_discussion(
        tx.request().clone(),
        tx.actor(),
        discussion_id_exists,
        command,
    )?;
    tx.save_created_discussion(&mutation, now_unix).await?;
    tx.commit().await?;
    Ok(mutation)
}

pub(crate) async fn persist_discussion_transition(
    store: &RequestStore,
    command: TransitionRequestDiscussionCommand,
) -> Result<RequestDiscussion, PostgresError> {
    let tx = store
        .begin_request(&command.request_id, &command.actor_user_id)
        .await?;
    if !tx
        .actor()
        .policy(tx.request(), &command.actor_user_id)
        .discussion_visible
    {
        return Err(PostgresError::not_found("request not found"));
    }
    let discussion = tx
        .discussion(&command.discussion_id)
        .await?
        .ok_or_else(|| PostgresError::not_found("request discussion not found"))?;
    let actor_user_id = command.actor_user_id.clone();
    let now_unix = command.now_unix;
    let input = RequestDiscussionTransitionInput {
        request_id: command.request_id,
        discussion_id: command.discussion_id,
        actor_user_id: command.actor_user_id,
        event_id: command.event_id,
        now_unix,
    };
    let mutation = match command.transition {
        DiscussionTransition::Resolve => {
            resolve_request_discussion(tx.request().clone(), tx.actor(), discussion, input)?
        }
        DiscussionTransition::Reopen => {
            reopen_request_discussion(tx.request().clone(), tx.actor(), discussion, input)?
        }
    };
    tx.save_discussion_transition(&mutation).await?;
    read_through(
        &tx,
        &mutation.discussion,
        &actor_user_id,
        mutation.discussion.last_activity_position,
        now_unix,
    )
    .await?;
    tx.commit().await?;
    Ok(mutation.discussion)
}

pub(crate) async fn persist_reply(
    store: &RequestStore,
    command: CreateRequestDiscussionReplyCommand,
) -> Result<CreateRequestDiscussionReplyMutation, PostgresError> {
    persist_reply_reopening(store, command, None).await
}

pub(crate) async fn persist_reopen_and_reply(
    store: &RequestStore,
    command: ReopenAndReplyToRequestDiscussionCommand,
) -> Result<CreateRequestDiscussionReplyMutation, PostgresError> {
    persist_reply_reopening(
        store,
        CreateRequestDiscussionReplyCommand {
            request_id: command.request_id,
            discussion_id: command.discussion_id,
            id: command.reply_id,
            actor_user_id: command.actor_user_id,
            client_reply_id: command.client_reply_id,
            body_markdown: command.body_markdown,
            reply_to_reply_id: command.reply_to_reply_id,
            wait_after_reply: command.wait_after_reply,
            now_unix: command.now_unix,
        },
        Some(command.event_id),
    )
    .await
}

async fn persist_reply_reopening(
    store: &RequestStore,
    command: CreateRequestDiscussionReplyCommand,
    reopen_event_id: Option<String>,
) -> Result<CreateRequestDiscussionReplyMutation, PostgresError> {
    let tx = store
        .begin_request(&command.request_id, &command.actor_user_id)
        .await?;
    let now_unix = command.now_unix;
    if reopen_event_id.is_some() {
        ensure_request_discussion_transition_allowed(
            tx.request(),
            tx.actor(),
            &command.actor_user_id,
        )?;
    }
    let discussion = tx
        .discussion(&command.discussion_id)
        .await?
        .ok_or_else(|| PostgresError::not_found("request discussion not found"))?;
    if let Some(reply) = tx
        .reply_by_client_id(&command.discussion_id, &command.client_reply_id)
        .await?
    {
        if !tx
            .actor()
            .policy(tx.request(), &command.actor_user_id)
            .discussion_visible
        {
            return Err(PostgresError::not_found("request discussion not found"));
        }
        let read_state = read_through(
            &tx,
            &discussion,
            &command.actor_user_id,
            reply.position,
            now_unix,
        )
        .await?;
        let request = tx.request().clone();
        tx.commit().await?;
        return Ok(CreateRequestDiscussionReplyMutation {
            created: false,
            request,
            discussion,
            reply,
            read_state,
            activity_event: None,
        });
    }
    let quoted_reply = match command.reply_to_reply_id.as_deref() {
        Some(quoted_id) => tx.reply(quoted_id).await?,
        None => None,
    };
    let wait_after_reply = command.wait_after_reply;
    let mutation = match reopen_event_id {
        Some(event_id) => reopen_and_reply_to_request_discussion(
            tx.request().clone(),
            tx.actor(),
            discussion,
            quoted_reply.as_ref(),
            ReopenAndReplyToRequestDiscussionInput {
                request_id: command.request_id,
                discussion_id: command.discussion_id,
                reply_id: command.id,
                actor_user_id: command.actor_user_id,
                event_id,
                client_reply_id: command.client_reply_id,
                body_markdown: command.body_markdown,
                reply_to_reply_id: command.reply_to_reply_id,
                now_unix,
            },
        )?,
        None => {
            let reply_id_exists = tx.reply(&command.id).await?.is_some();
            create_request_discussion_reply(
                tx.request().clone(),
                tx.actor(),
                discussion,
                quoted_reply.as_ref(),
                reply_id_exists,
                CreateRequestDiscussionReplyInput {
                    request_id: command.request_id,
                    discussion_id: command.discussion_id,
                    id: command.id,
                    actor_user_id: command.actor_user_id,
                    client_reply_id: command.client_reply_id,
                    body_markdown: command.body_markdown,
                    reply_to_reply_id: command.reply_to_reply_id,
                    now_unix,
                },
            )?
        }
    };
    tx.save_discussion_reply(&mutation, wait_after_reply, now_unix)
        .await?;
    tx.commit().await?;
    Ok(mutation)
}

pub(crate) async fn persist_discussion_read(
    store: &RequestStore,
    input: MarkRequestDiscussionReadInput,
) -> Result<RequestDiscussionReadState, PostgresError> {
    let (tx, discussion) = store
        .begin_request_discussion_read(&input.discussion_id, &input.user_id)
        .await?;
    if !tx
        .actor()
        .policy(tx.request(), &input.user_id)
        .discussion_visible
    {
        return Err(PostgresError::not_found("request not found"));
    }
    let existing = tx.discussion_read_state(&input.discussion_id).await?;
    let state = mark_request_discussion_read(&discussion, existing, input)?;
    tx.save_discussion_read_state(&state).await?;
    tx.commit().await?;
    Ok(state)
}

async fn read_through(
    tx: &RequestTransaction,
    discussion: &RequestDiscussion,
    user_id: &str,
    position: u64,
    now_unix: u64,
) -> Result<RequestDiscussionReadState, PostgresError> {
    let existing = tx.discussion_read_state(&discussion.id).await?;
    let state = mark_request_discussion_read(
        discussion,
        existing,
        MarkRequestDiscussionReadInput {
            discussion_id: discussion.id.clone(),
            user_id: user_id.to_string(),
            through_position: position,
            now_unix,
        },
    )?;
    tx.save_discussion_read_state(&state).await?;
    Ok(state)
}

#[cfg(test)]
mod tests;
