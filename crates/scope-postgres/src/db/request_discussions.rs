use super::request_discussion_commands::{
    CreateRequestDiscussionCommand, CreateRequestDiscussionReplyCommand, DiscussionTransition,
    ReopenAndReplyToRequestDiscussionCommand, TransitionRequestDiscussionCommand,
};
use super::{
    RequestStore,
    data_scope::DataScope,
    request_access::{ensure_user_exists, lock_request_repository, request_policy_for_user},
    request_discussion_rows::{
        discussion_by_client_id, discussion_by_id, insert_discussion, insert_reply, read_state,
        reply_by_client_id, reply_by_id, save_discussion, save_read_state,
    },
    request_media::replace_bindings_for_markdown,
    request_revision_rows::revision_by_id,
    request_rows::insert_request_event_row,
    request_rows::save_request_row,
};
use {
    crate::error::PostgresError,
    scope_domain::requests::{
        CreateRequestDiscussionInput, CreateRequestDiscussionMutation,
        CreateRequestDiscussionReplyInput, CreateRequestDiscussionReplyMutation,
        MarkRequestDiscussionReadInput, ReopenAndReplyToRequestDiscussionInput, RequestDiscussion,
        RequestDiscussionReadState, RequestDiscussionTransitionInput, RequestRevision,
        create_request_discussion, create_request_discussion_reply,
        ensure_request_discussion_transition_allowed, mark_request_discussion_read,
        reopen_and_reply_to_request_discussion, reopen_request_discussion,
        resolve_request_discussion,
    },
};

impl RequestStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "request_revision"))]
    pub async fn request_revision(
        &self,
        request_id: &str,
        revision_id: &str,
    ) -> Result<Option<RequestRevision>, PostgresError> {
        let tx = self
            .db
            .begin_read_snapshot(DataScope::RequestRepository(request_id))
            .await?;
        let revision = revision_by_id(&tx, revision_id)
            .await?
            .filter(|revision| revision.request_id == request_id);
        tx.commit().await?;
        Ok(revision)
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "create_request_discussion"))]
    pub async fn create_request_discussion(
        &self,
        command: CreateRequestDiscussionCommand,
    ) -> Result<CreateRequestDiscussionMutation, PostgresError> {
        let tx = self
            .db
            .begin(DataScope::RequestRepository(&command.request_id))
            .await?;
        let (repo, request) =
            lock_request_repository(&tx, &command.request_id, &command.actor_user_id).await?;
        ensure_user_exists(&tx, &command.actor_user_id).await?;
        let policy = request_policy_for_user(&tx, &repo, &request, &command.actor_user_id).await?;
        let now_unix = command.now_unix;
        let input = CreateRequestDiscussionInput {
            request_id: command.request_id,
            id: command.id,
            actor_user_id: command.actor_user_id,
            actor_can_participate: policy.permissions.can_open_discussion,
            client_discussion_id: command.client_discussion_id,
            body_markdown: command.body_markdown,
            anchor: command.anchor,
            now_unix: command.now_unix,
        };

        if let Some(discussion) = discussion_by_client_id(
            &tx,
            &input.request_id,
            &input.actor_user_id,
            &input.client_discussion_id,
        )
        .await?
        {
            if !policy.discussion_visible {
                return Err(PostgresError::not_found("request discussion not found"));
            }
            let state = match read_state(&tx, &discussion.id, &input.actor_user_id).await? {
                Some(state) => state,
                None => {
                    monotonic_read_state(
                        &tx,
                        &discussion,
                        &input.actor_user_id,
                        discussion.opened_position,
                        input.now_unix,
                    )
                    .await?
                }
            };
            tx.commit().await?;
            return Ok(CreateRequestDiscussionMutation {
                created: false,
                request,
                discussion,
                read_state: state,
            });
        }

        let discussion_id_exists = discussion_by_id(&tx, &input.id).await?.is_some();
        let actor_user_id = input.actor_user_id.clone();
        let mutation = create_request_discussion(request, discussion_id_exists, input)?;
        save_request_row(&tx, &mutation.request).await?;
        insert_discussion(&tx, &mutation.discussion).await?;
        save_read_state(&tx, &mutation.read_state).await?;
        super::request_attention::reactivate_attention_for_activity(
            &tx,
            &mutation.request.id,
            &actor_user_id,
            mutation.request.activity_version,
            mutation.discussion.created_at_unix,
        )
        .await?;
        let discussion = &mutation.discussion;
        replace_bindings_for_markdown(
            &tx,
            &discussion.request_id,
            &actor_user_id,
            &scope_domain::requests::attachments::RequestAttachmentBindingTarget::Discussion {
                discussion_id: discussion.id.clone(),
            },
            &discussion.body_markdown,
            now_unix,
        )
        .await?;
        tx.commit().await?;
        Ok(mutation)
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "create_request_discussion_reply"))]
    pub async fn create_request_discussion_reply(
        &self,
        command: CreateRequestDiscussionReplyCommand,
    ) -> Result<CreateRequestDiscussionReplyMutation, PostgresError> {
        self.persist_request_discussion_reply(command, None).await
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "transition_request_discussion"))]
    pub async fn transition_request_discussion(
        &self,
        command: TransitionRequestDiscussionCommand,
    ) -> Result<RequestDiscussion, PostgresError> {
        let TransitionRequestDiscussionCommand {
            request_id,
            discussion_id,
            actor_user_id,
            event_id,
            now_unix,
            transition,
        } = command;
        let tx = self
            .db
            .begin(DataScope::RequestRepository(&request_id))
            .await?;
        let (repo, request) = lock_request_repository(&tx, &request_id, &actor_user_id).await?;
        ensure_user_exists(&tx, &actor_user_id).await?;
        let policy = request_policy_for_user(&tx, &repo, &request, &actor_user_id).await?;
        if !policy.discussion_visible {
            return Err(PostgresError::not_found("request not found"));
        }
        let discussion = discussion_by_id(&tx, &discussion_id)
            .await?
            .filter(|discussion| discussion.request_id == request_id)
            .ok_or_else(|| PostgresError::not_found("request discussion not found"))?;
        let actor_is_maintainer = repo.access.is_maintainer();
        let actor = actor_user_id.clone();
        let mutation = match transition {
            DiscussionTransition::Resolve => resolve_request_discussion(
                request,
                discussion,
                RequestDiscussionTransitionInput {
                    request_id,
                    discussion_id,
                    actor_user_id,
                    actor_is_maintainer,
                    actor_can_transition: policy.permissions.can_transition_discussion,
                    event_id,
                    now_unix,
                },
            )?,
            DiscussionTransition::Reopen => reopen_request_discussion(
                request,
                discussion,
                RequestDiscussionTransitionInput {
                    request_id,
                    discussion_id,
                    actor_user_id,
                    actor_is_maintainer,
                    actor_can_transition: policy.permissions.can_transition_discussion,
                    event_id,
                    now_unix,
                },
            )?,
        };
        save_request_row(&tx, &mutation.request).await?;
        save_discussion(&tx, &mutation.discussion).await?;
        insert_request_event_row(&tx, &mutation.event).await?;
        monotonic_read_state(
            &tx,
            &mutation.discussion,
            &actor,
            mutation.discussion.last_activity_position,
            now_unix,
        )
        .await?;
        tx.commit().await?;
        Ok(mutation.discussion)
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "reopen_and_reply_to_request_discussion"))]
    pub async fn reopen_and_reply_to_request_discussion(
        &self,
        command: ReopenAndReplyToRequestDiscussionCommand,
    ) -> Result<CreateRequestDiscussionReplyMutation, PostgresError> {
        let ReopenAndReplyToRequestDiscussionCommand {
            request_id,
            discussion_id,
            reply_id,
            actor_user_id,
            event_id,
            client_reply_id,
            body_markdown,
            reply_to_reply_id,
            wait_after_reply,
            now_unix,
        } = command;
        self.persist_request_discussion_reply(
            CreateRequestDiscussionReplyCommand {
                request_id,
                discussion_id,
                id: reply_id,
                actor_user_id,
                client_reply_id,
                body_markdown,
                reply_to_reply_id,
                wait_after_reply,
                now_unix,
            },
            Some(event_id),
        )
        .await
    }

    async fn persist_request_discussion_reply(
        &self,
        command: CreateRequestDiscussionReplyCommand,
        reopen_event_id: Option<String>,
    ) -> Result<CreateRequestDiscussionReplyMutation, PostgresError> {
        let tx = self
            .db
            .begin(DataScope::RequestRepository(&command.request_id))
            .await?;
        let (repo, request) =
            lock_request_repository(&tx, &command.request_id, &command.actor_user_id).await?;
        ensure_user_exists(&tx, &command.actor_user_id).await?;
        let policy = request_policy_for_user(&tx, &repo, &request, &command.actor_user_id).await?;
        let actor_is_maintainer = repo.access.is_maintainer();
        let wait_after_reply = command.wait_after_reply;
        let now_unix = command.now_unix;
        if reopen_event_id.is_some() {
            ensure_request_discussion_transition_allowed(
                &request,
                policy.permissions.can_transition_discussion,
            )?;
        }
        let discussion = discussion_by_id(&tx, &command.discussion_id)
            .await?
            .filter(|discussion| discussion.request_id == command.request_id)
            .ok_or_else(|| PostgresError::not_found("request discussion not found"))?;
        if let Some(reply) = reply_by_client_id(
            &tx,
            &command.discussion_id,
            &command.actor_user_id,
            &command.client_reply_id,
        )
        .await?
        {
            if !policy.discussion_visible {
                return Err(PostgresError::not_found("request discussion not found"));
            }
            let state = monotonic_read_state(
                &tx,
                &discussion,
                &command.actor_user_id,
                reply.position,
                command.now_unix,
            )
            .await?;
            tx.commit().await?;
            return Ok(CreateRequestDiscussionReplyMutation {
                created: false,
                request,
                discussion,
                reply,
                read_state: state,
                activity_event: None,
            });
        }
        let actor_user_id = command.actor_user_id.clone();
        let quoted_reply = match command.reply_to_reply_id.as_deref() {
            Some(quoted_id) => reply_by_id(&tx, quoted_id).await?,
            None => None,
        };
        let mutation = match reopen_event_id {
            Some(event_id) => reopen_and_reply_to_request_discussion(
                request,
                discussion,
                quoted_reply.as_ref(),
                ReopenAndReplyToRequestDiscussionInput {
                    request_id: command.request_id,
                    discussion_id: command.discussion_id,
                    reply_id: command.id,
                    actor_user_id: command.actor_user_id,
                    actor_is_maintainer,
                    actor_can_transition: policy.permissions.can_transition_discussion,
                    actor_can_participate: policy.permissions.can_reply_to_discussion,
                    event_id,
                    client_reply_id: command.client_reply_id,
                    body_markdown: command.body_markdown,
                    reply_to_reply_id: command.reply_to_reply_id,
                    now_unix: command.now_unix,
                },
            )?,
            None => {
                let reply_id_exists = reply_by_id(&tx, &command.id).await?.is_some();
                create_request_discussion_reply(
                    request,
                    discussion,
                    quoted_reply.as_ref(),
                    reply_id_exists,
                    CreateRequestDiscussionReplyInput {
                        request_id: command.request_id,
                        discussion_id: command.discussion_id,
                        id: command.id,
                        actor_user_id: command.actor_user_id,
                        actor_can_participate: policy.permissions.can_reply_to_discussion,
                        client_reply_id: command.client_reply_id,
                        body_markdown: command.body_markdown,
                        reply_to_reply_id: command.reply_to_reply_id,
                        now_unix: command.now_unix,
                    },
                )?
            }
        };
        save_request_row(&tx, &mutation.request).await?;
        save_discussion(&tx, &mutation.discussion).await?;
        insert_reply(&tx, &mutation.reply).await?;
        save_read_state(&tx, &mutation.read_state).await?;
        if let Some(event) = &mutation.activity_event {
            insert_request_event_row(&tx, event).await?;
        }
        super::request_attention::reactivate_attention_for_activity(
            &tx,
            &mutation.request.id,
            &actor_user_id,
            mutation.request.activity_version,
            mutation.reply.created_at_unix,
        )
        .await?;
        if wait_after_reply {
            super::request_attention::wait_after_own_reply(
                &tx,
                &mutation.request,
                &actor_user_id,
                actor_is_maintainer,
                mutation.reply.position,
                mutation.reply.created_at_unix,
            )
            .await?;
        }
        let reply = &mutation.reply;
        replace_bindings_for_markdown(
            &tx,
            &mutation.discussion.request_id,
            &actor_user_id,
            &scope_domain::requests::attachments::RequestAttachmentBindingTarget::Reply {
                discussion_id: reply.discussion_id.clone(),
                reply_id: reply.id.clone(),
            },
            &reply.body_markdown,
            now_unix,
        )
        .await?;
        tx.commit().await?;
        Ok(mutation)
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "mark_request_discussion_read"))]
    pub async fn mark_request_discussion_read(
        &self,
        input: MarkRequestDiscussionReadInput,
    ) -> Result<RequestDiscussionReadState, PostgresError> {
        let tx = self
            .db
            .begin(DataScope::DiscussionRepository(&input.discussion_id))
            .await?;
        ensure_user_exists(&tx, &input.user_id).await?;
        let discussion = discussion_by_id(&tx, &input.discussion_id)
            .await?
            .ok_or_else(|| PostgresError::not_found("request discussion not found"))?;
        let (repo, request) =
            lock_request_repository(&tx, &discussion.request_id, &input.user_id).await?;
        if !request_policy_for_user(&tx, &repo, &request, &input.user_id)
            .await?
            .discussion_visible
        {
            return Err(PostgresError::not_found("request not found"));
        }
        let existing_state = read_state(&tx, &input.discussion_id, &input.user_id).await?;
        let state = mark_request_discussion_read(&discussion, existing_state, input)?;
        save_read_state(&tx, &state).await?;
        tx.commit().await?;
        Ok(state)
    }
}

pub(super) async fn monotonic_read_state<C>(
    conn: &C,
    discussion: &RequestDiscussion,
    user_id: &str,
    position: u64,
    now_unix: u64,
) -> Result<RequestDiscussionReadState, PostgresError>
where
    C: sea_orm::ConnectionTrait,
{
    let existing_state = read_state(conn, &discussion.id, user_id).await?;
    let state = mark_request_discussion_read(
        discussion,
        existing_state,
        MarkRequestDiscussionReadInput {
            discussion_id: discussion.id.clone(),
            user_id: user_id.to_string(),
            through_position: position,
            now_unix,
        },
    )?;
    save_read_state(conn, &state).await?;
    Ok(state)
}
