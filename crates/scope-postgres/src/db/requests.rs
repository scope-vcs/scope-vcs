use super::{
    CloseRequestCommand, EditRequestIdentityCommand, RequestStore, acquire_aggregate_lock,
    cleanup_queue::queue::queue_pending_source_blob_deletion_rows,
    object_references::delete_object_reference,
    request_access::{
        authorize_start_request, ensure_user_exists, lock_request_repository, repo_by_id,
        request_policy_for_user,
    },
    request_auto_merge::{
        automatic_event_id, lock_active_intent_for_request, persist_existing_auto_merge_mutation,
    },
    request_lifecycle_effects::persist_lifecycle_mutation,
    request_media::{replace_bindings_for_markdown, tombstone_request_attachments},
    request_revision_rows::{insert_revision, revisions_for_request_ids},
    request_rows::{
        delete_request_rows, insert_request_event_row, insert_request_row, latest_request_events,
        public_draft_count, request_by_id, request_by_name, request_event_by_id,
        request_events_after_position, request_events_by_request_id, request_list_page,
        requests_by_repo_id, save_request_row,
    },
};
use sea_orm::{EntityTrait, QuerySelect, TransactionTrait};
use {
    crate::error::PostgresError,
    scope_domain::content::SourceBlob,
    scope_domain::requests::{
        CloseRequestInput, CloseRequestMutation, EditRequestIdentityInput,
        RecordRequestRevisionInput, RecordWorkingRequestUploadInput, Request,
        RequestAutoMergeStopReason, RequestEvent, RequestRevision, RequestRevisionMutation,
        RequestTimelineMutation, StartRequestFacts, StartRequestInput, StartRequestMutation,
        WorkingRequestUploadMutation, close_request, edit_request_identity,
        record_request_revision, record_working_request_upload, start_request,
        stop_request_auto_merge,
    },
};

impl RequestStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "request_list_page"))]
    pub async fn request_list_page(
        &self,
        input: super::RequestListPageQuery<'_>,
    ) -> Result<Vec<super::RequestListRow>, PostgresError> {
        request_list_page(self.db.as_ref(), input).await
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "request_by_id"))]
    pub async fn request_by_id(&self, request_id: &str) -> Result<Option<Request>, PostgresError> {
        request_by_id(self.db.as_ref(), request_id).await
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "request_by_name"))]
    pub async fn request_by_name(
        &self,
        repo_id: &str,
        request_name: &str,
    ) -> Result<Option<Request>, PostgresError> {
        request_by_name(self.db.as_ref(), repo_id, request_name).await
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "requests_by_repo_id"))]
    pub async fn requests_by_repo_id(&self, repo_id: &str) -> Result<Vec<Request>, PostgresError> {
        requests_by_repo_id(self.db.as_ref(), repo_id).await
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "request_events_by_request_id"))]
    pub async fn request_events_by_request_id(
        &self,
        request_id: &str,
    ) -> Result<Vec<RequestEvent>, PostgresError> {
        request_events_by_request_id(self.db.as_ref(), request_id).await
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "request_events_after_position"))]
    pub async fn request_events_after_position(
        &self,
        request_id: &str,
        after_position: u64,
        limit: u64,
    ) -> Result<Vec<RequestEvent>, PostgresError> {
        request_events_after_position(self.db.as_ref(), request_id, after_position, limit).await
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "latest_request_events"))]
    pub async fn latest_request_events(
        &self,
        request_id: &str,
        limit: u64,
    ) -> Result<Vec<RequestEvent>, PostgresError> {
        latest_request_events(self.db.as_ref(), request_id, limit).await
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "start_request"))]
    pub async fn start_request(
        &self,
        input: StartRequestInput,
    ) -> Result<StartRequestMutation, PostgresError> {
        let tx = self.db.begin().await?;
        acquire_aggregate_lock(&tx, "repository", &input.repo_id).await?;
        let mut graph_ids = super::request_stack::ancestry(&tx, &input.base)
            .await?
            .into_iter()
            .map(|request| request.id)
            .collect::<std::collections::BTreeSet<_>>();
        graph_ids.insert(input.id.clone());
        for id in graph_ids {
            acquire_aggregate_lock(&tx, "request", &id).await?;
        }
        ensure_user_exists(&tx, &input.author_user_id).await?;
        let repo = repo_by_id(&tx, &input.repo_id, &input.author_user_id).await?;
        let mut input = authorize_start_request(&repo, input)?;
        let stack_depth = super::request_stack::validate_start(&tx, &mut input, &repo).await?;

        let facts = StartRequestFacts {
            stack_depth,
            request_id_exists: request_by_id(&tx, &input.id).await?.is_some(),
            request_name_exists: request_by_name(&tx, &input.repo_id, &input.name)
                .await?
                .is_some(),
            public_working_request_count: public_draft_count(
                &tx,
                &input.repo_id,
                &input.author_user_id,
            )
            .await?,
        };
        let mut mutation = start_request(facts, input, &repo.views)?;
        if let Some(parent) = super::request_stack::ancestry(&tx, &mutation.request.base)
            .await?
            .first()
        {
            mutation.request = scope_domain::requests::refresh_request_stack(
                mutation.request,
                Some(parent),
                scope_domain::requests::RequestParentGitFacts {
                    parent_contains_pin: true,
                    ancestor_blocked: parent.stack_status.requires_restack(),
                    integration_conflict: false,
                },
                mutation.event.created_at_unix,
            )?
            .request;
        }
        insert_request_row(&tx, &mutation.request).await?;
        insert_request_event_row(&tx, &mutation.event).await?;
        tx.commit().await?;
        Ok(mutation)
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "record_working_request_upload"))]
    pub async fn record_working_request_upload(
        &self,
        input: RecordWorkingRequestUploadInput,
    ) -> Result<WorkingRequestUploadMutation, PostgresError> {
        let tx = self.db.begin().await?;
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
        queue_pending_source_blob_deletion_rows(&tx, mutation.orphan_objects.clone(), now_unix)
            .await?;
        tx.commit().await?;
        Ok(mutation)
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "record_request_revision"))]
    pub async fn record_request_revision(
        &self,
        input: RecordRequestRevisionInput,
    ) -> Result<RequestRevisionMutation, PostgresError> {
        let observed = self
            .request_by_id(&input.request_id)
            .await?
            .ok_or_else(|| PostgresError::not_found("request not found"))?;
        self.record_request_revision_with_stack(input, observed.base_version, None, Vec::new())
            .await
    }

    pub async fn record_request_revision_with_stack(
        &self,
        input: RecordRequestRevisionInput,
        expected_base_version: u64,
        facts: Option<super::RequestBasePublicationFacts>,
        children: Vec<super::RequestStackChildFacts>,
    ) -> Result<RequestRevisionMutation, PostgresError> {
        self.publish_request_revision(input, expected_base_version, facts, children, None, None)
            .await
    }

    pub async fn record_pushed_request_revision(
        &self,
        input: RecordRequestRevisionInput,
        expected_base_version: u64,
        facts: Option<super::RequestBasePublicationFacts>,
        children: Vec<super::RequestStackChildFacts>,
        objects: PushedRevisionObjects,
    ) -> Result<RequestRevisionMutation, PostgresError> {
        self.publish_request_revision(
            input,
            expected_base_version,
            facts,
            children,
            None,
            Some(objects),
        )
        .await
    }

    pub async fn record_request_revision_context(
        &self,
        input: RecordRequestRevisionInput,
        context: super::RequestRevisionContextPublication,
        facts: Option<super::RequestBasePublicationFacts>,
        children: Vec<super::RequestStackChildFacts>,
    ) -> Result<RequestRevisionMutation, PostgresError> {
        self.publish_request_revision(
            input,
            context.expected_base_version,
            facts,
            children,
            Some(context),
            None,
        )
        .await
    }

    async fn publish_request_revision(
        &self,
        input: RecordRequestRevisionInput,
        expected_base_version: u64,
        facts: Option<super::RequestBasePublicationFacts>,
        children: Vec<super::RequestStackChildFacts>,
        context: Option<super::RequestRevisionContextPublication>,
        pushed: Option<PushedRevisionObjects>,
    ) -> Result<RequestRevisionMutation, PostgresError> {
        let observed = self
            .request_by_id(&input.request_id)
            .await?
            .ok_or_else(|| PostgresError::not_found("request not found"))?;
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        super::locks::acquire_shared_repository_lock(&tx, &observed.repo_id).await?;
        let request = request_by_id(&tx, &input.request_id)
            .await?
            .filter(|request| request.repo_id == observed.repo_id)
            .ok_or_else(|| PostgresError::not_found("request not found"))?;
        let repo = repo_by_id(&tx, &request.repo_id, &input.actor_user_id).await?;
        if let Some(pushed) = &pushed {
            ensure_object_generation(&tx, &request.repo_id, &pushed.object_generation).await?;
        }
        super::request_stack::lock_graph(&tx, &request, input.new_base.as_ref()).await?;
        let request = request_by_id(&tx, &input.request_id)
            .await?
            .ok_or_else(|| PostgresError::not_found("request not found"))?;
        if let Some(context) = &context
            && let Some(accepted) = super::request_revision_context::receipt(
                &tx,
                &repo,
                &request,
                &input.actor_user_id,
                context,
            )
            .await?
        {
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(accepted);
        }
        if let Some(context) = &context {
            super::request_revision_context::validate(
                &tx,
                &request,
                &input,
                context,
                facts.as_ref(),
            )
            .await?;
        }
        let publication = super::request_stack::validate_publication(
            &tx,
            &request,
            &input,
            expected_base_version,
            facts.as_ref(),
            &repo,
        )
        .await?;
        ensure_user_exists(&tx, &input.actor_user_id).await?;
        let mut input = input;
        input.new_base_stack_depth = match input.new_base.as_ref() {
            Some(scope_domain::requests::RequestBase::Request { .. }) => Some(publication.depth),
            _ => None,
        };
        let now_unix = input.now_unix;
        input.actor_can_edit = request_policy_for_user(&tx, &repo, &request, &input.actor_user_id)
            .await?
            .branch_mutable;
        let event_id_exists = request_event_by_id(&tx, &input.event_id).await?.is_some();
        let active_auto_merge = lock_active_intent_for_request(&tx, &request.id).await?;
        let actor_user_id = input.actor_user_id.clone();
        let mut mutation = record_request_revision(request, event_id_exists, input)?;
        if let Some(parent) = publication.parent.as_ref() {
            let evidence = parent.stack_status.requires_restack();
            let integration_conflict = mutation.request.stack_status
                == scope_domain::requests::RequestStackStatus::IntegrationConflict;
            let refreshed = scope_domain::requests::refresh_request_stack(
                mutation.request,
                Some(parent),
                scope_domain::requests::RequestParentGitFacts {
                    parent_contains_pin: true,
                    ancestor_blocked: evidence,
                    integration_conflict,
                },
                now_unix,
            )?;
            mutation.request = refreshed.request;
        }
        save_request_row(&tx, &mutation.request).await?;
        insert_request_event_row(&tx, &mutation.event).await?;
        insert_revision(&tx, &mutation.revision).await?;
        if let Some(pack) = pushed.as_ref().and_then(|pushed| pushed.pool_pack.as_ref()) {
            if pack.request_id != mutation.request.id || pack.head_oid != mutation.request.head_oid
            {
                return Err(PostgresError::internal_message(
                    "Git pool pack does not belong to the recorded revision",
                ));
            }
            super::git_segments::register_git_pool_pack(&tx, &repo.record.id, pack, now_unix)
                .await?;
        }
        super::request_checks::clear_current_evaluation(&tx, &mutation.request.id).await?;
        let mutation = if let Some(stored) = active_auto_merge {
            let stopped = stop_request_auto_merge(
                &mutation.request,
                &stored.intent,
                RequestAutoMergeStopReason::RequestChanged,
                automatic_event_id("stopped", &stored.intent.id),
                now_unix,
            )?;
            persist_existing_auto_merge_mutation(&tx, stored.model, &stopped).await?;
            RequestRevisionMutation {
                request: stopped.request,
                ..mutation
            }
        } else {
            mutation
        };
        super::request_attention::reactivate_attention_for_activity(
            &tx,
            &mutation.request.id,
            &actor_user_id,
            mutation.request.activity_version,
            mutation.revision.created_at_unix,
        )
        .await?;
        queue_pending_source_blob_deletion_rows(&tx, mutation.orphan_objects.clone(), now_unix)
            .await?;
        super::request_stack::refresh_children(&tx, &mutation.request, &children, now_unix).await?;
        if let Some(context) = &context {
            super::request_revision_context::save_receipt(&tx, context, &mutation).await?;
        }
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(mutation)
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "edit_request_identity"))]
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
        let tx = self.db.begin().await?;
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
        tx.commit().await?;
        Ok(mutation)
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "close_request"))]
    pub async fn close_request(
        &self,
        command: CloseRequestCommand,
    ) -> Result<CloseRequestMutation, PostgresError> {
        let now_unix = command.now_unix;
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let observed = request_by_id(&tx, &command.request_id)
            .await?
            .ok_or_else(|| PostgresError::not_found("request not found"))?;
        acquire_aggregate_lock(&tx, "repository", &observed.repo_id).await?;
        super::request_stack::lock_graph(&tx, &observed, None).await?;
        let (repo, request) =
            lock_request_repository(&tx, &command.request_id, &command.actor_user_id).await?;
        ensure_user_exists(&tx, &command.actor_user_id).await?;
        let events = request_events_by_request_id(&tx, &request.id).await?;
        let revisions = revisions_for_request_ids(&tx, std::slice::from_ref(&request.id)).await?;
        let active_auto_merge = lock_active_intent_for_request(&tx, &request.id).await?;
        let input = CloseRequestInput {
            request_id: command.request_id,
            actor_user_id: command.actor_user_id,
            actor_is_maintainer: repo.access.is_maintainer(),
            event_id: command.event_id,
            now_unix,
        };
        let mut mutation = close_request(request, events, revisions, input)?;
        match &mut mutation {
            CloseRequestMutation::DeletedDraft {
                request,
                revisions,
                orphan_objects,
                ..
            } => {
                persist_deleted_draft(&tx, request, revisions, orphan_objects.clone(), now_unix)
                    .await?;
            }
            CloseRequestMutation::Closed { request, event } => {
                persist_lifecycle_mutation(&tx, request, std::slice::from_ref(event)).await?;
                super::request_stack::close_children(&tx, request, now_unix).await?;
                if let Some(stored) = active_auto_merge {
                    let stopped = stop_request_auto_merge(
                        request,
                        &stored.intent,
                        RequestAutoMergeStopReason::RequestClosed,
                        automatic_event_id("stopped", &stored.intent.id),
                        now_unix,
                    )?;
                    persist_existing_auto_merge_mutation(&tx, stored.model, &stopped).await?;
                    *request = stopped.request;
                }
            }
        }
        tx.commit().await?;
        Ok(mutation)
    }
}

#[derive(Clone, Debug)]
pub struct PushedRevisionObjects {
    pub object_generation: String,
    pub pool_pack: Option<scope_domain::repository::git::GitPoolPack>,
}

async fn ensure_object_generation<C: sea_orm::ConnectionTrait>(
    tx: &C,
    repo_id: &str,
    expected: &str,
) -> Result<(), PostgresError> {
    let config = super::entities::repository::Entity::find_by_id(repo_id)
        .select_only()
        .column(super::entities::repository::Column::RepoConfig)
        .into_tuple::<serde_json::Value>()
        .one(tx)
        .await?
        .ok_or_else(|| PostgresError::not_found("repo not found"))?;
    let config: scope_domain::repo_config::RepoConfig =
        serde_json::from_value(config).map_err(PostgresError::internal)?;
    if config.object_generation() == expected {
        Ok(())
    } else {
        Err(PostgresError::conflict(
            "repository visibility changed during the push; retry",
        ))
    }
}

pub(super) async fn persist_deleted_draft<C: sea_orm::ConnectionTrait>(
    tx: &C,
    request: &Request,
    revisions: &[RequestRevision],
    orphan_objects: Vec<SourceBlob>,
    now_unix: u64,
) -> Result<(), PostgresError> {
    super::git_segments::retire_request_pool_packs(tx, &request.id, now_unix).await?;
    super::github_pushes::queue_github_branch_deletion(tx, &request.repo_id, &request.id, now_unix)
        .await?;
    tombstone_request_attachments(tx, &request.id, now_unix).await?;
    for revision in revisions {
        delete_object_reference(tx, "request_revision_snapshot", &revision.id).await?;
    }
    delete_request_rows(tx, &request.id).await?;
    queue_pending_source_blob_deletion_rows(tx, orphan_objects, now_unix).await
}

#[cfg(test)]
pub(super) mod tests;
