use super::{RequestStore, entities, request_access::request_actor, request_rows::request_by_id};
use crate::error::PostgresError;
use scope_domain::{
    repository::access::RepositoryAccessContext,
    requests::{
        EditRequestLinksInput, LinkedRequest, NAMED_REQUEST_NOT_FOUND, NamedRequest, Request,
        RequestActor, RequestLink, RequestLinksMutation, StoredRequestLinks, edit_request_links,
        ensure_named_requests_within_limit,
    },
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, ConnectionTrait, EntityTrait, IntoActiveModel,
    QueryFilter, QuerySelect,
};
use std::collections::BTreeSet;

impl RequestStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "edit_request_links"))]
    pub async fn edit_request_links(
        &self,
        request_id: &str,
        actor_user_id: &str,
        resolve: &[String],
        stop_resolving: &[String],
        now_unix: u64,
    ) -> Result<RequestLinksMutation, PostgresError> {
        let tx = self.begin_request(request_id, actor_user_id).await?;
        let mutation = edit_request_links(
            tx.request(),
            tx.actor(),
            &tx.resolves().await?,
            EditRequestLinksInput {
                actor_user_id: actor_user_id.to_string(),
                resolve: tx.named_requests(resolve).await?,
                stop_resolving: tx.named_requests(stop_resolving).await?,
                now_unix,
            },
        )?;
        tx.save_links(&mutation).await?;
        tx.commit().await?;
        Ok(mutation)
    }
}

pub(super) async fn named_requests<C: ConnectionTrait>(
    conn: &C,
    repository: &RepositoryAccessContext,
    request_ids: &[String],
    actor_user_id: &str,
) -> Result<Vec<NamedRequest>, PostgresError> {
    ensure_named_requests_within_limit(request_ids.len())?;
    let mut named = Vec::with_capacity(request_ids.len());
    for request_id in request_ids {
        let request = request_by_id(conn, request_id)
            .await?
            .ok_or_else(|| PostgresError::not_found(NAMED_REQUEST_NOT_FOUND))?;
        let actor = request_actor(conn, repository, &request, actor_user_id).await?;
        named.push(NamedRequest::seen_by_actor(&request, actor, actor_user_id)?);
    }
    Ok(named)
}

pub(super) async fn link_started_request<C: ConnectionTrait>(
    conn: &C,
    repository: &RepositoryAccessContext,
    request: &Request,
    actor_user_id: &str,
    resolves: &[String],
) -> Result<(), PostgresError> {
    if resolves.is_empty() {
        return Ok(());
    }
    let mutation = edit_request_links(
        request,
        RequestActor {
            access: &repository.access,
            is_invitee: false,
            views: &repository.views,
        },
        &[],
        EditRequestLinksInput {
            actor_user_id: actor_user_id.to_string(),
            resolve: named_requests(conn, repository, resolves, actor_user_id).await?,
            stop_resolving: Vec::new(),
            now_unix: request.created_at_unix,
        },
    )?;
    save_links(conn, &mutation).await
}

pub(super) async fn resolves<C: ConnectionTrait>(
    conn: &C,
    request_id: &str,
) -> Result<Vec<RequestLink>, PostgresError> {
    links(
        conn,
        Condition::all().add(entities::request_link::Column::ResolvingRequestId.eq(request_id)),
    )
    .await
}

async fn links<C: ConnectionTrait>(
    conn: &C,
    condition: Condition,
) -> Result<Vec<RequestLink>, PostgresError> {
    entities::request_link::Entity::find()
        .filter(condition)
        .all(conn)
        .await?
        .into_iter()
        .map(entities::request_link::Model::try_into_domain)
        .collect()
}

pub(super) async fn save_links<C: ConnectionTrait>(
    conn: &C,
    mutation: &RequestLinksMutation,
) -> Result<(), PostgresError> {
    for link in &mutation.removed {
        entities::request_link::Entity::delete_by_id((
            link.resolving_request_id.clone(),
            link.resolved_request_id.clone(),
        ))
        .exec(conn)
        .await?;
    }
    for link in &mutation.added {
        entities::request_link::Model::from_domain(link)?
            .into_active_model()
            .insert(conn)
            .await?;
    }
    Ok(())
}

pub(super) async fn stored_links<C: ConnectionTrait>(
    conn: &C,
    request: &Request,
    viewer_user_id: Option<&str>,
) -> Result<StoredRequestLinks, PostgresError> {
    let links = links(
        conn,
        Condition::any()
            .add(entities::request_link::Column::ResolvingRequestId.eq(&request.id))
            .add(entities::request_link::Column::ResolvedRequestId.eq(&request.id)),
    )
    .await?;
    let linked_ids = links
        .iter()
        .flat_map(|link| [&link.resolving_request_id, &link.resolved_request_id])
        .map(String::as_str)
        .chain(request.close_named_request_id())
        .filter(|id| *id != request.id)
        .map(str::to_string)
        .collect::<BTreeSet<_>>();
    if linked_ids.is_empty() {
        return Ok(StoredRequestLinks::default());
    }
    let invited = match viewer_user_id {
        Some(viewer_user_id) => entities::request_invitee::Entity::find()
            .select_only()
            .column(entities::request_invitee::Column::RequestId)
            .filter(entities::request_invitee::Column::RequestId.is_in(linked_ids.iter().cloned()))
            .filter(entities::request_invitee::Column::UserId.eq(viewer_user_id))
            .into_tuple::<String>()
            .all(conn)
            .await?
            .into_iter()
            .collect(),
        None => BTreeSet::new(),
    };
    let requests = entities::request::Entity::find()
        .filter(entities::request::Column::Id.is_in(linked_ids))
        .all(conn)
        .await?
        .into_iter()
        .map(|model| {
            let request = model.try_into_domain()?;
            Ok(LinkedRequest {
                viewer_is_invitee: invited.contains(&request.id),
                request,
            })
        })
        .collect::<Result<_, PostgresError>>()?;
    Ok(StoredRequestLinks { links, requests })
}

#[cfg(test)]
mod tests;
