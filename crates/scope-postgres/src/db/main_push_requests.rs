use super::{
    RequestStore, acquire_aggregate_lock, entities,
    request_access::{ensure_user_exists, repo_by_id},
    request_revision_rows::insert_revision,
    request_rows::{
        insert_request_event_row, insert_request_row, public_draft_count, request_by_id,
        request_by_name,
    },
};
use crate::error::PostgresError;
use scope_domain::requests::{
    MainPushRequestMutation, StartMainPushRequestInput, StartRequestFacts, main_push_request_name,
    start_main_push_request,
};
use sea_orm::{ActiveModelTrait, IntoActiveModel, TransactionTrait};

impl RequestStore {
    pub async fn start_main_push_request(
        &self,
        input: StartMainPushRequestInput,
    ) -> Result<MainPushRequestMutation, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        acquire_aggregate_lock(&tx, "repository", &input.repo_id).await?;
        acquire_aggregate_lock(&tx, "request", &input.id).await?;
        ensure_user_exists(&tx, &input.pusher_user_id).await?;
        let repo = repo_by_id(&tx, &input.repo_id, &input.pusher_user_id).await?;
        if repo.record.incarnation_id != input.repository_incarnation_id {
            return Err(PostgresError::conflict(
                "repository was recreated since the push started",
            ));
        }
        let facts = StartRequestFacts {
            request_id_exists: request_by_id(&tx, &input.id).await?.is_some(),
            request_name_exists: request_by_name(
                &tx,
                &input.repo_id,
                &main_push_request_name(&input.head_oid),
            )
            .await?
            .is_some(),
            public_working_request_count: public_draft_count(
                &tx,
                &input.repo_id,
                &input.pusher_user_id,
            )
            .await?,
        };
        let mutation = start_main_push_request(
            facts,
            &repo.access,
            repo.record.lifecycle_state,
            input,
            &repo.views,
        )?;
        insert_request_row(&tx, &mutation.request).await?;
        for event in &mutation.events {
            insert_request_event_row(&tx, event).await?;
        }
        insert_revision(&tx, &mutation.revision).await?;
        let auto_merge_position = mutation
            .events
            .last()
            .map(|event| event.position)
            .ok_or_else(|| PostgresError::internal_message("main push request has no events"))?;
        entities::request_auto_merge_intent::Model::from_domain(
            &mutation.auto_merge,
            auto_merge_position,
        )?
        .into_active_model()
        .insert(&tx)
        .await
        .map_err(PostgresError::internal)?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(mutation)
    }
}
