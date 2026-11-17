use super::{
    RequestStore, acquire_aggregate_lock,
    github_pull_requests::{queue_carrier_for_pushed_head, requeue_carrier},
    github_pushes::latest_github_push,
    request_access::{ensure_user_exists, lock_request_repository},
    request_auto_merge::{
        StoredIntent, lock_active_intent_for_request, persist_existing_auto_merge_mutation,
    },
    request_discussion_rows::{
        discussion_by_client_id, discussion_by_id, insert_discussion, insert_reply, read_state,
        reply_by_client_id, reply_by_id, save_discussion, save_read_state,
    },
    request_invitees::request_is_invitee,
    request_lifecycle_effects::{persist_deleted_draft, persist_lifecycle_mutation},
    request_media::replace_bindings_for_markdown,
    request_revision_rows::revisions_for_request_ids,
    request_rows::{
        insert_request_event_row, request_by_id, request_event_by_id, request_events_by_request_id,
        save_request_row,
    },
};
use crate::error::PostgresError;
use scope_domain::{
    repository::access::RepositoryAccessContext,
    requests::{
        CloseRequestMutation, CreateRequestDiscussionMutation,
        CreateRequestDiscussionReplyMutation, GitHubPush, Request, RequestActor,
        RequestAutoMergeIntent, RequestAutoMergeMutation, RequestDiscussion,
        RequestDiscussionMutation, RequestDiscussionReadState, RequestDiscussionReply,
        RequestEvent, RequestLifecycleMutation, RequestRevision, RequestTimelineMutation,
        attachments::RequestAttachmentBindingTarget,
    },
};
use sea_orm::{DatabaseTransaction, TransactionTrait};

pub struct RequestTransaction {
    tx: DatabaseTransaction,
    repository: RepositoryAccessContext,
    request: Request,
    actor_user_id: String,
    actor_is_invitee: bool,
}

pub struct RequestCloseFacts {
    pub events: Vec<RequestEvent>,
    pub revisions: Vec<RequestRevision>,
    pub active_auto_merge: Option<LockedAutoMerge>,
}

pub struct LockedAutoMerge(StoredIntent);

impl LockedAutoMerge {
    pub fn intent(&self) -> &RequestAutoMergeIntent {
        &self.0.intent
    }
}

impl RequestStore {
    pub async fn begin_request(
        &self,
        request_id: &str,
        actor_user_id: &str,
    ) -> Result<RequestTransaction, PostgresError> {
        let tx = self.db.begin().await?;
        RequestTransaction::lock(tx, request_id, actor_user_id).await
    }

    pub async fn begin_request_close(
        &self,
        request_id: &str,
        actor_user_id: &str,
    ) -> Result<RequestTransaction, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let observed = request_by_id(&tx, request_id)
            .await?
            .ok_or_else(|| PostgresError::not_found("request not found"))?;
        acquire_aggregate_lock(&tx, "repository", &observed.repo_id).await?;
        super::request_stack::lock_graph(&tx, &observed, None).await?;
        RequestTransaction::lock(tx, request_id, actor_user_id).await
    }

    pub async fn begin_request_discussion_read(
        &self,
        discussion_id: &str,
        user_id: &str,
    ) -> Result<(RequestTransaction, RequestDiscussion), PostgresError> {
        let tx = self.db.begin().await?;
        ensure_user_exists(&tx, user_id).await?;
        let discussion = discussion_by_id(&tx, discussion_id)
            .await?
            .ok_or_else(|| PostgresError::not_found("request discussion not found"))?;
        let (repository, request) =
            lock_request_repository(&tx, &discussion.request_id, user_id).await?;
        let transaction = RequestTransaction::loaded(tx, repository, request, user_id).await?;
        Ok((transaction, discussion))
    }
}

impl RequestTransaction {
    async fn lock(
        tx: DatabaseTransaction,
        request_id: &str,
        actor_user_id: &str,
    ) -> Result<Self, PostgresError> {
        let (repository, request) = lock_request_repository(&tx, request_id, actor_user_id).await?;
        ensure_user_exists(&tx, actor_user_id).await?;
        Self::loaded(tx, repository, request, actor_user_id).await
    }

    async fn loaded(
        tx: DatabaseTransaction,
        repository: RepositoryAccessContext,
        request: Request,
        actor_user_id: &str,
    ) -> Result<Self, PostgresError> {
        let actor_is_invitee = request_is_invitee(&tx, &request.id, actor_user_id).await?;
        Ok(Self {
            tx,
            repository,
            request,
            actor_user_id: actor_user_id.to_string(),
            actor_is_invitee,
        })
    }

    pub fn request(&self) -> &Request {
        &self.request
    }

    pub fn repository(&self) -> &RepositoryAccessContext {
        &self.repository
    }

    pub fn actor(&self) -> RequestActor<'_> {
        RequestActor {
            access: &self.repository.access,
            is_invitee: self.actor_is_invitee,
            views: &self.repository.views,
        }
    }

    pub async fn commit(self) -> Result<(), PostgresError> {
        Ok(self.tx.commit().await?)
    }

    pub async fn event_id_exists(&self, event_id: &str) -> Result<bool, PostgresError> {
        Ok(request_event_by_id(&self.tx, event_id).await?.is_some())
    }

    pub async fn save_submission(
        &self,
        mutation: &RequestLifecycleMutation,
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        persist_lifecycle_mutation(&self.tx, &mutation.request, &mutation.events).await?;
        if let Some(pushed_head) = latest_github_push(&self.tx, &mutation.request.id)
            .await?
            .as_ref()
            .and_then(GitHubPush::delivered_head)
        {
            queue_carrier_for_pushed_head(&self.tx, &mutation.request.id, pushed_head, now_unix)
                .await?;
        }
        Ok(())
    }

    pub async fn save_identity_edit(
        &self,
        mutation: &RequestTimelineMutation,
        new_description_markdown: Option<&str>,
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        save_request_row(&self.tx, &mutation.request).await?;
        insert_request_event_row(&self.tx, &mutation.event).await?;
        requeue_carrier(
            &self.tx,
            &mutation.request.id,
            mutation.request.updated_at_unix,
        )
        .await?;
        if let Some(markdown) = new_description_markdown {
            self.bind_attachments(
                &mutation.request.id,
                &RequestAttachmentBindingTarget::Description,
                markdown,
                now_unix,
            )
            .await?;
        }
        Ok(())
    }

    pub async fn close_facts(&self) -> Result<RequestCloseFacts, PostgresError> {
        Ok(RequestCloseFacts {
            events: request_events_by_request_id(&self.tx, &self.request.id).await?,
            revisions: revisions_for_request_ids(&self.tx, std::slice::from_ref(&self.request.id))
                .await?,
            active_auto_merge: lock_active_intent_for_request(&self.tx, &self.request.id)
                .await?
                .map(LockedAutoMerge),
        })
    }

    pub async fn save_close(
        &self,
        mutation: &CloseRequestMutation,
        stopped_auto_merge: Option<(LockedAutoMerge, &RequestAutoMergeMutation)>,
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        match mutation {
            CloseRequestMutation::DeletedDraft { request, .. } => {
                persist_deleted_draft(&self.tx, request, now_unix).await
            }
            CloseRequestMutation::Closed { request, event } => {
                persist_lifecycle_mutation(&self.tx, request, std::slice::from_ref(event)).await?;
                super::request_stack::close_children(&self.tx, request, now_unix).await?;
                if let Some((locked, stopped)) = stopped_auto_merge {
                    persist_existing_auto_merge_mutation(&self.tx, locked.0.model, stopped).await?;
                }
                Ok(())
            }
        }
    }

    pub async fn discussion(
        &self,
        discussion_id: &str,
    ) -> Result<Option<RequestDiscussion>, PostgresError> {
        Ok(discussion_by_id(&self.tx, discussion_id)
            .await?
            .filter(|discussion| discussion.request_id == self.request.id))
    }

    pub async fn discussion_id_exists(&self, discussion_id: &str) -> Result<bool, PostgresError> {
        Ok(discussion_by_id(&self.tx, discussion_id).await?.is_some())
    }

    pub async fn discussion_by_client_id(
        &self,
        client_discussion_id: &str,
    ) -> Result<Option<RequestDiscussion>, PostgresError> {
        discussion_by_client_id(
            &self.tx,
            &self.request.id,
            &self.actor_user_id,
            client_discussion_id,
        )
        .await
    }

    pub async fn reply(
        &self,
        reply_id: &str,
    ) -> Result<Option<RequestDiscussionReply>, PostgresError> {
        reply_by_id(&self.tx, reply_id).await
    }

    pub async fn reply_by_client_id(
        &self,
        discussion_id: &str,
        client_reply_id: &str,
    ) -> Result<Option<RequestDiscussionReply>, PostgresError> {
        reply_by_client_id(
            &self.tx,
            discussion_id,
            &self.actor_user_id,
            client_reply_id,
        )
        .await
    }

    pub async fn discussion_read_state(
        &self,
        discussion_id: &str,
    ) -> Result<Option<RequestDiscussionReadState>, PostgresError> {
        read_state(&self.tx, discussion_id, &self.actor_user_id).await
    }

    pub async fn save_discussion_read_state(
        &self,
        state: &RequestDiscussionReadState,
    ) -> Result<(), PostgresError> {
        save_read_state(&self.tx, state).await
    }

    pub async fn save_created_discussion(
        &self,
        mutation: &CreateRequestDiscussionMutation,
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        save_request_row(&self.tx, &mutation.request).await?;
        insert_discussion(&self.tx, &mutation.discussion).await?;
        save_read_state(&self.tx, &mutation.read_state).await?;
        super::request_attention::reactivate_attention_for_activity(
            &self.tx,
            &mutation.request.id,
            &self.actor_user_id,
            mutation.request.activity_version,
            mutation.discussion.created_at_unix,
        )
        .await?;
        self.bind_attachments(
            &mutation.discussion.request_id,
            &RequestAttachmentBindingTarget::Discussion {
                discussion_id: mutation.discussion.id.clone(),
            },
            &mutation.discussion.body_markdown,
            now_unix,
        )
        .await
    }

    pub async fn save_discussion_reply(
        &self,
        mutation: &CreateRequestDiscussionReplyMutation,
        wait_after_reply: bool,
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        save_request_row(&self.tx, &mutation.request).await?;
        save_discussion(&self.tx, &mutation.discussion).await?;
        insert_reply(&self.tx, &mutation.reply).await?;
        save_read_state(&self.tx, &mutation.read_state).await?;
        if let Some(event) = &mutation.activity_event {
            insert_request_event_row(&self.tx, event).await?;
        }
        super::request_attention::reactivate_attention_for_activity(
            &self.tx,
            &mutation.request.id,
            &self.actor_user_id,
            mutation.request.activity_version,
            mutation.reply.created_at_unix,
        )
        .await?;
        if wait_after_reply {
            super::request_attention::wait_after_own_reply(
                &self.tx,
                &mutation.request,
                &self.actor_user_id,
                self.repository.access.is_maintainer(),
                mutation.reply.position,
                mutation.reply.created_at_unix,
            )
            .await?;
        }
        self.bind_attachments(
            &mutation.discussion.request_id,
            &RequestAttachmentBindingTarget::Reply {
                discussion_id: mutation.reply.discussion_id.clone(),
                reply_id: mutation.reply.id.clone(),
            },
            &mutation.reply.body_markdown,
            now_unix,
        )
        .await
    }

    pub async fn save_discussion_transition(
        &self,
        mutation: &RequestDiscussionMutation,
    ) -> Result<(), PostgresError> {
        save_request_row(&self.tx, &mutation.request).await?;
        save_discussion(&self.tx, &mutation.discussion).await?;
        insert_request_event_row(&self.tx, &mutation.event).await
    }

    async fn bind_attachments(
        &self,
        request_id: &str,
        target: &RequestAttachmentBindingTarget,
        markdown: &str,
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        replace_bindings_for_markdown(
            &self.tx,
            request_id,
            &self.actor_user_id,
            target,
            markdown,
            now_unix,
        )
        .await
    }
}
