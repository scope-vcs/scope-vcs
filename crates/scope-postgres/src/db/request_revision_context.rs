use super::{
    RequestStore, entities,
    request_access::{lock_request_repository, request_policy_for_user},
};
use crate::error::PostgresError;
use scope_domain::requests::{
    RecordRequestRevisionInput, Request, RequestBase, RequestRevisionMutation,
};
use sea_orm::{
    ActiveModelTrait, ConnectionTrait, DatabaseTransaction, EntityTrait, Set, TransactionTrait,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Serialize)]
pub struct RequestRevisionContextPublication {
    pub operation_key: String,
    pub expected_revision_id: String,
    pub expected_head_oid: String,
    pub expected_base_version: u64,
    pub base: RequestBase,
    pub expected_parent_base_version: u64,
}

impl RequestStore {
    pub async fn request_revision_context_receipt(
        &self,
        request_id: &str,
        actor_user_id: &str,
        context: &RequestRevisionContextPublication,
    ) -> Result<Option<RequestRevisionMutation>, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let (repo, request) = lock_request_repository(&tx, request_id, actor_user_id).await?;
        let result = receipt(&tx, &repo, &request, actor_user_id, context).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(result)
    }
}

fn digest(context: &RequestRevisionContextPublication) -> Result<String, PostgresError> {
    let payload = serde_json::to_vec(context).map_err(PostgresError::internal)?;
    Ok(Sha256::digest(payload)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

pub(super) async fn receipt<C: ConnectionTrait>(
    conn: &C,
    repo: &scope_domain::repository::access::RepositoryAccessContext,
    request: &Request,
    actor_user_id: &str,
    context: &RequestRevisionContextPublication,
) -> Result<Option<RequestRevisionMutation>, PostgresError> {
    if !request_policy_for_user(conn, repo, request, actor_user_id)
        .await?
        .branch_mutable
    {
        return Err(PostgresError::permission_denied(
            "request branch edit permission required",
        ));
    }
    let Some(row) = entities::revision_context_receipt::Entity::find_by_id((
        request.id.clone(),
        context.operation_key.clone(),
    ))
    .one(conn)
    .await
    .map_err(PostgresError::internal)?
    else {
        return Ok(None);
    };
    if row.payload_digest != digest(context)? {
        return Err(PostgresError::conflict(
            "revision-context operation key has a different payload",
        ));
    }
    let revision = super::request_revision_rows::revision_by_id(conn, &row.revision_id)
        .await?
        .ok_or_else(|| {
            PostgresError::internal_message("accepted revision-context revision is missing")
        })?;
    let event = super::request_rows::request_event_by_id(conn, &row.revision_id)
        .await?
        .ok_or_else(|| {
            PostgresError::internal_message("accepted revision-context event is missing")
        })?;
    Ok(Some(RequestRevisionMutation {
        request: request.clone(),
        event,
        revision,
        orphan_objects: Vec::new(),
    }))
}

pub(super) async fn validate(
    tx: &DatabaseTransaction,
    request: &Request,
    input: &RecordRequestRevisionInput,
    context: &RequestRevisionContextPublication,
    parent: Option<&super::RequestParentPublicationFacts>,
) -> Result<(), PostgresError> {
    let revision = super::request_revision_rows::latest_revision_for_request(tx, &request.id)
        .await?
        .ok_or_else(|| PostgresError::conflict("request has no saved revision"))?;
    if context.operation_key.trim().is_empty()
        || context.operation_key.len() > 200
        || revision.id != context.expected_revision_id
        || request.head_oid != context.expected_head_oid
        || input.new_head_oid != context.expected_head_oid
        || input.expected_old_head_oid.as_deref() != Some(context.expected_head_oid.as_str())
        || request.base_version != context.expected_base_version
        || input.new_base.as_ref() != Some(&context.base)
        || input.git_snapshot != revision.git_snapshot
        || parent.is_none_or(|parent| parent.base_version != context.expected_parent_base_version)
    {
        return Err(PostgresError::conflict(
            "request changed during revision-context publication",
        ));
    }
    Ok(())
}

pub(super) async fn save_receipt(
    tx: &DatabaseTransaction,
    context: &RequestRevisionContextPublication,
    mutation: &RequestRevisionMutation,
) -> Result<(), PostgresError> {
    entities::revision_context_receipt::ActiveModel {
        request_id: Set(mutation.request.id.clone()),
        operation_key: Set(context.operation_key.clone()),
        payload_digest: Set(digest(context)?),
        revision_id: Set(mutation.revision.id.clone()),
        created_at_unix: Set(super::integer_columns::u64_to_i64(
            mutation.revision.created_at_unix,
            "receipt creation time",
        )?),
    }
    .insert(tx)
    .await
    .map_err(PostgresError::internal)?;
    Ok(())
}
