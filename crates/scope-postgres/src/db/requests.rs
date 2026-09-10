use super::{
    CloseRequestCommand, EditRequestIdentityCommand, GeneratedIdSource, RequestStore,
    acquire_aggregate_lock,
    cleanup_queue::queue::queue_pending_source_blob_deletion_rows,
    object_references::delete_object_reference,
    request_access::{
        authorize_start_request, ensure_user_exists, lock_request_repository, repo_by_id,
        request_policy_for_user,
    },
    request_invitees::delete_request_invitees,
    request_media::{replace_bindings_for_markdown, tombstone_request_attachments},
    request_revision_rows::{insert_revision, revisions_for_request_ids},
    request_rows::{
        delete_request_rows, insert_request_event_row, insert_request_row, latest_request_events,
        request_by_id, request_by_name, request_event_by_id, request_events_after_position,
        request_events_by_request_id, request_list_page, requests_by_repo_author,
        requests_by_repo_id, save_request_row,
    },
};
use sea_orm::TransactionTrait;
use std::sync::Arc;
use {
    crate::error::PostgresError,
    scope_domain::requests::{
        CloseRequestInput, CloseRequestMutation, EditRequestIdentityInput,
        RecordRequestRevisionInput, RecordWorkingRequestUploadInput, Request, RequestActorRole,
        RequestEvent, RequestRevisionMutation, RequestState, RequestTimelineMutation,
        StartRequestFacts, StartRequestInput, StartRequestMutation, WorkingRequestUploadMutation,
        close_request, edit_request_identity, record_request_revision,
        record_working_request_upload, start_request,
    },
};

impl RequestStore {
    pub async fn request_list_page(
        &self,
        input: super::RequestListPageQuery<'_>,
    ) -> Result<Vec<super::RequestListRow>, PostgresError> {
        request_list_page(self.db.as_ref(), input).await
    }

    pub async fn request_by_id(&self, request_id: &str) -> Result<Option<Request>, PostgresError> {
        let request_id = request_id.to_string();
        let db = Arc::clone(&self.db);
        request_by_id(db.as_ref(), &request_id).await
    }

    pub async fn request_by_name(
        &self,
        repo_id: &str,
        request_name: &str,
    ) -> Result<Option<Request>, PostgresError> {
        let repo_id = repo_id.to_string();
        let request_name = request_name.to_string();
        let db = Arc::clone(&self.db);
        request_by_name(db.as_ref(), &repo_id, &request_name).await
    }

    pub async fn requests_by_repo_id(&self, repo_id: &str) -> Result<Vec<Request>, PostgresError> {
        let repo_id = repo_id.to_string();
        let db = Arc::clone(&self.db);
        requests_by_repo_id(db.as_ref(), &repo_id).await
    }

    pub async fn requests_by_repo_author(
        &self,
        repo_id: &str,
        author_user_id: &str,
    ) -> Result<Vec<Request>, PostgresError> {
        let repo_id = repo_id.to_string();
        let author_user_id = author_user_id.to_string();
        let db = Arc::clone(&self.db);
        requests_by_repo_author(db.as_ref(), &repo_id, &author_user_id).await
    }

    pub async fn request_events_by_request_id(
        &self,
        request_id: &str,
    ) -> Result<Vec<RequestEvent>, PostgresError> {
        let request_id = request_id.to_string();
        let db = Arc::clone(&self.db);
        request_events_by_request_id(db.as_ref(), &request_id).await
    }

    pub async fn request_events_after_position(
        &self,
        request_id: &str,
        after_position: u64,
        limit: u64,
    ) -> Result<Vec<RequestEvent>, PostgresError> {
        request_events_after_position(self.db.as_ref(), request_id, after_position, limit).await
    }

    pub async fn latest_request_events(
        &self,
        request_id: &str,
        limit: u64,
    ) -> Result<Vec<RequestEvent>, PostgresError> {
        latest_request_events(self.db.as_ref(), request_id, limit).await
    }

    pub async fn start_request(
        &self,
        input: StartRequestInput,
    ) -> Result<StartRequestMutation, PostgresError> {
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(PostgresError::internal)?;
        acquire_aggregate_lock(&tx, "repository", &input.repo_id).await?;
        acquire_aggregate_lock(&tx, "request", &input.id).await?;
        ensure_user_exists(&tx, &input.author_user_id).await?;
        let input = authorize_start_request(
            &repo_by_id(&tx, &input.repo_id, &input.author_user_id).await?,
            input,
        )?;

        let author_requests =
            requests_by_repo_author(&tx, &input.repo_id, &input.author_user_id).await?;
        let facts = StartRequestFacts {
            request_id_exists: request_by_id(&tx, &input.id).await?.is_some(),
            request_name_exists: request_by_name(&tx, &input.repo_id, &input.name)
                .await?
                .is_some(),
            public_working_request_count: author_requests
                .iter()
                .filter(|request| {
                    request.author_role == RequestActorRole::Public
                        && request.state() == RequestState::Draft
                })
                .count(),
        };
        let mutation = start_request(facts, input)?;
        insert_request_row(&tx, &mutation.request).await?;
        insert_request_event_row(&tx, &mutation.event).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(mutation)
    }

    pub async fn record_working_request_upload(
        &self,
        input: RecordWorkingRequestUploadInput,
        generated_ids: &dyn GeneratedIdSource,
    ) -> Result<WorkingRequestUploadMutation, PostgresError> {
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(PostgresError::internal)?;
        let (repo, request) =
            lock_request_repository(&tx, &input.request_id, &input.actor_user_id).await?;
        ensure_user_exists(&tx, &input.actor_user_id).await?;
        let mut input = input;
        let now_unix = input.now_unix;
        input.actor_can_edit = request_policy_for_user(&tx, &repo, &request, &input.actor_user_id)
            .await?
            .branch_mutable;
        let mutation = record_working_request_upload(request, input)?;
        save_request_row(&tx, &mutation.request).await?;
        if !mutation.orphan_objects.is_empty() {
            queue_pending_source_blob_deletion_rows(
                &tx,
                mutation.orphan_objects.clone(),
                now_unix,
                generated_ids,
            )
            .await?;
        }
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(mutation)
    }

    pub async fn record_request_revision(
        &self,
        input: RecordRequestRevisionInput,
        generated_ids: &dyn GeneratedIdSource,
    ) -> Result<RequestRevisionMutation, PostgresError> {
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(PostgresError::internal)?;
        let (repo, request) =
            lock_request_repository(&tx, &input.request_id, &input.actor_user_id).await?;
        ensure_user_exists(&tx, &input.actor_user_id).await?;
        let mut input = input;
        let now_unix = input.now_unix;
        input.actor_can_edit = request_policy_for_user(&tx, &repo, &request, &input.actor_user_id)
            .await?
            .branch_mutable;
        let event_id_exists = request_event_by_id(&tx, &input.event_id).await?.is_some();
        let mutation = record_request_revision(request, event_id_exists, input)?;
        save_request_row(&tx, &mutation.request).await?;
        insert_request_event_row(&tx, &mutation.event).await?;
        insert_revision(&tx, &mutation.revision).await?;
        super::request_attention::reactivate_attention_for_activity(
            &tx,
            &mutation.request.id,
            &mutation.revision.actor_user_id,
            mutation.request.activity_version,
            mutation.revision.created_at_unix,
        )
        .await?;
        if !mutation.orphan_objects.is_empty() {
            queue_pending_source_blob_deletion_rows(
                &tx,
                mutation.orphan_objects.clone(),
                now_unix,
                generated_ids,
            )
            .await?;
        }
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(mutation)
    }

    pub async fn edit_request_identity(
        &self,
        command: EditRequestIdentityCommand,
    ) -> Result<RequestTimelineMutation, PostgresError> {
        let attachment_binding = command.description_markdown.as_ref().map(|markdown| {
            (
                command.request_id.clone(),
                command.actor_user_id.clone(),
                markdown.clone(),
                command.now_unix,
            )
        });
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(PostgresError::internal)?;
        let (repo, request) =
            lock_request_repository(&tx, &command.request_id, &command.actor_user_id).await?;
        ensure_user_exists(&tx, &command.actor_user_id).await?;
        let actor_can_edit_identity =
            request_policy_for_user(&tx, &repo, &request, &command.actor_user_id)
                .await?
                .permissions
                .can_edit_identity;
        let event_id_exists = request_event_by_id(&tx, &command.event_id).await?.is_some();
        let mutation = edit_request_identity(
            request,
            event_id_exists,
            EditRequestIdentityInput {
                request_id: command.request_id,
                actor_user_id: command.actor_user_id,
                actor_can_edit_identity,
                event_id: command.event_id,
                title: command.title,
                description_markdown: command.description_markdown,
                expected_description_markdown: command.expected_description_markdown,
                now_unix: command.now_unix,
            },
        )?;
        save_request_row(&tx, &mutation.request).await?;
        insert_request_event_row(&tx, &mutation.event).await?;
        if let Some((request_id, actor_user_id, markdown, now_unix)) = attachment_binding {
            replace_bindings_for_markdown(
                &tx,
                &request_id,
                &actor_user_id,
                &scope_domain::requests::attachments::RequestAttachmentBindingTarget::Description,
                &markdown,
                now_unix,
            )
            .await?;
        }
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(mutation)
    }

    pub async fn close_request(
        &self,
        command: CloseRequestCommand,
        generated_ids: &dyn GeneratedIdSource,
    ) -> Result<CloseRequestMutation, PostgresError> {
        let db = Arc::clone(&self.db);
        let now_unix = command.now_unix;
        let tx = db.as_ref().begin().await.map_err(PostgresError::internal)?;
        let (repo, request) =
            lock_request_repository(&tx, &command.request_id, &command.actor_user_id).await?;
        ensure_user_exists(&tx, &command.actor_user_id).await?;
        let events = request_events_by_request_id(&tx, &request.id).await?;
        let revisions = revisions_for_request_ids(&tx, std::slice::from_ref(&request.id)).await?;
        let input = CloseRequestInput {
            request_id: command.request_id,
            actor_is_author: request.author_user_id == command.actor_user_id,
            actor_user_id: command.actor_user_id,
            actor_is_maintainer: repo.access.is_maintainer(),
            event_id: command.event_id,
            now_unix,
        };
        let mutation = close_request(request, events, revisions, input)?;
        match &mutation {
            CloseRequestMutation::DeletedDraft {
                request,
                revisions,
                orphan_objects,
                ..
            } => {
                tombstone_request_attachments(&tx, &request.id, now_unix).await?;
                for revision in revisions {
                    delete_object_reference(&tx, "request_revision_snapshot", &revision.id).await?;
                }
                delete_request_rows(&tx, &request.id).await?;
                if !orphan_objects.is_empty() {
                    queue_pending_source_blob_deletion_rows(
                        &tx,
                        orphan_objects.clone(),
                        now_unix,
                        generated_ids,
                    )
                    .await?;
                }
            }
            CloseRequestMutation::Closed { request, event } => {
                save_request_row(&tx, request).await?;
                delete_request_invitees(&tx, &request.id).await?;
                insert_request_event_row(&tx, event).await?;
            }
        }
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(mutation)
    }
}

#[cfg(test)]
pub(super) mod tests;
