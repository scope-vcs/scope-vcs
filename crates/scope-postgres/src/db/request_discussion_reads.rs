use super::{
    RequestStore,
    data_scope::DataScope,
    request_discussion_actors::{DiscussionActorIds, RequestDiscussionActors},
    request_discussion_rows::{
        DiscussionPageFilter, RequestDiscussionReplyReadModel,
        RequestDiscussionReplyReferenceReadModel, changed_discussions_for_request,
        discussion_by_id, discussions_page_for_request, read_states_for_user,
        replies_for_discussion, reply_by_id, reply_previews_for_discussions, unread_content_counts,
    },
    request_revision_rows::{
        RequestRevisionWindow, revision_positions_for_request, revision_window_for_request,
    },
};
use crate::error::PostgresError;
use scope_domain::requests::RequestDiscussion;
use sea_orm::DatabaseTransaction;
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct RequestDiscussionReadModel {
    pub discussion: RequestDiscussion,
    pub anchor_revision_position: Option<u64>,
    pub reply_count: u64,
    pub latest_replies: Vec<RequestDiscussionReplyReadModel>,
    pub read_through_position: u64,
    pub unread_count: u64,
}

#[derive(Clone, Debug)]
pub struct RequestDiscussionReadBatch {
    pub discussions: Vec<RequestDiscussionReadModel>,
    pub actors: RequestDiscussionActors,
}

#[derive(Clone, Debug)]
pub struct RequestDiscussionsPageQuery<'a> {
    pub request_id: &'a str,
    pub viewer_user_id: Option<&'a str>,
    pub snapshot_version: u64,
    pub cursor: Option<(u64, String)>,
    pub discussion_id: Option<&'a str>,
    pub anchor_revision_id: Option<&'a str>,
    pub anchor_commit_oid: Option<&'a str>,
    pub include_revision_anchor: bool,
    pub include_carried_line_anchors: bool,
    pub limit: u64,
}

pub type DiscussionWithActors = (RequestDiscussionReadModel, RequestDiscussionActors);
pub type ReplyWithActors = (RequestDiscussionReplyReadModel, RequestDiscussionActors);

impl RequestStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "request_revision_window"))]
    pub async fn request_revision_window(
        &self,
        request_id: &str,
        selected_revision_id: Option<&str>,
        limit: u64,
    ) -> Result<RequestRevisionWindow, PostgresError> {
        let tx = self
            .db
            .begin_read_snapshot(DataScope::RequestRepository(request_id))
            .await?;
        let window =
            revision_window_for_request(&tx, request_id, selected_revision_id, limit).await?;
        tx.commit().await?;
        Ok(window)
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "request_discussions_page"))]
    pub async fn request_discussions_page(
        &self,
        query: RequestDiscussionsPageQuery<'_>,
    ) -> Result<RequestDiscussionReadBatch, PostgresError> {
        let tx = self
            .db
            .begin_read_snapshot(DataScope::RequestRepository(query.request_id))
            .await?;
        let discussions = discussions_page_for_request(
            &tx,
            query.request_id,
            query.snapshot_version,
            query.cursor,
            DiscussionPageFilter {
                discussion_id: query.discussion_id,
                revision_id: query.anchor_revision_id,
                commit_oid: query.anchor_commit_oid,
                include_revision_anchor: query.include_revision_anchor,
                include_carried_line_anchors: query.include_carried_line_anchors,
            },
            query.limit,
        )
        .await?;
        let batch = hydrate_discussions(&tx, discussions, query.viewer_user_id).await?;
        tx.commit().await?;
        Ok(batch)
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "request_discussion"))]
    pub async fn request_discussion(
        &self,
        request_id: &str,
        discussion_id: &str,
        viewer_user_id: Option<&str>,
    ) -> Result<Option<DiscussionWithActors>, PostgresError> {
        let tx = self
            .db
            .begin_read_snapshot(DataScope::RequestRepository(request_id))
            .await?;
        let discussion = match discussion_by_id(&tx, discussion_id).await? {
            Some(discussion) if discussion.request_id == request_id => discussion,
            _ => return Ok(None),
        };
        let mut batch = hydrate_discussions(&tx, vec![discussion], viewer_user_id).await?;
        tx.commit().await?;
        Ok(batch
            .discussions
            .pop()
            .map(|discussion| (discussion, batch.actors)))
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "changed_request_discussions"))]
    pub async fn changed_request_discussions(
        &self,
        request_id: &str,
        viewer_user_id: Option<&str>,
        after_position: u64,
        limit: u64,
    ) -> Result<RequestDiscussionReadBatch, PostgresError> {
        let tx = self
            .db
            .begin_read_snapshot(DataScope::RequestRepository(request_id))
            .await?;
        let discussions =
            changed_discussions_for_request(&tx, request_id, after_position, limit).await?;
        let batch = hydrate_discussions(&tx, discussions, viewer_user_id).await?;
        tx.commit().await?;
        Ok(batch)
    }
}

async fn hydrate_discussions(
    tx: &DatabaseTransaction,
    discussions: Vec<RequestDiscussion>,
    viewer_user_id: Option<&str>,
) -> Result<RequestDiscussionReadBatch, PostgresError> {
    let revision_positions = match discussions
        .iter()
        .find(|discussion| discussion.anchor.is_some())
    {
        Some(discussion) => revision_positions_for_request(tx, &discussion.request_id).await?,
        None => BTreeMap::new(),
    };
    let ids = discussions
        .iter()
        .map(|discussion| discussion.id.clone())
        .collect::<Vec<_>>();
    let read_states = match viewer_user_id {
        Some(user_id) => read_states_for_user(tx, &ids, user_id).await?,
        None => BTreeMap::new(),
    };
    let previews = reply_previews_for_discussions(tx, &ids).await?;
    let unread_counts = match viewer_user_id {
        Some(_) => unread_content_counts(tx, &discussions, &read_states).await?,
        None => BTreeMap::new(),
    };
    let mut actor_ids = DiscussionActorIds::default();
    for discussion in &discussions {
        actor_ids.author(&discussion.author);
        actor_ids.resolver(discussion.resolved_by.as_ref());
    }
    let mut models = Vec::with_capacity(discussions.len());
    for discussion in discussions {
        let anchor_revision_position = discussion
            .anchor
            .as_ref()
            .map(|anchor| {
                revision_positions
                    .get(&anchor.revision_id)
                    .copied()
                    .ok_or_else(|| {
                        PostgresError::internal_message(format!(
                            "request discussion anchor references unknown revision {}",
                            anchor.revision_id
                        ))
                    })
            })
            .transpose()?;
        let (reply_count, latest_replies) =
            previews.get(&discussion.id).cloned().unwrap_or_default();
        for model in &latest_replies {
            reply_actor_ids(&mut actor_ids, model);
        }
        let read_through_position = read_states
            .get(&discussion.id)
            .map(|state| state.read_through_position)
            .unwrap_or(0);
        let unread_count = unread_counts.get(&discussion.id).copied().unwrap_or(0);
        models.push(RequestDiscussionReadModel {
            discussion,
            anchor_revision_position,
            reply_count,
            latest_replies,
            read_through_position,
            unread_count,
        });
    }
    Ok(RequestDiscussionReadBatch {
        discussions: models,
        actors: actor_ids.load(tx).await?,
    })
}

impl RequestStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "request_discussion_replies"))]
    pub async fn request_discussion_replies(
        &self,
        discussion_id: &str,
        before_position: Option<u64>,
        limit: u64,
    ) -> Result<
        (
            Vec<RequestDiscussionReplyReadModel>,
            RequestDiscussionActors,
        ),
        PostgresError,
    > {
        let tx = self
            .db
            .begin_read_snapshot(DataScope::DiscussionRepository(discussion_id))
            .await?;
        let replies = replies_for_discussion(&tx, discussion_id, before_position, limit).await?;
        let mut actor_ids = DiscussionActorIds::default();
        for model in &replies {
            reply_actor_ids(&mut actor_ids, model);
        }
        let actors = actor_ids.load(&tx).await?;
        tx.commit().await?;
        Ok((replies, actors))
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "request_discussion_reply"))]
    pub async fn request_discussion_reply(
        &self,
        discussion_id: &str,
        reply_id: &str,
    ) -> Result<Option<ReplyWithActors>, PostgresError> {
        let tx = self
            .db
            .begin_read_snapshot(DataScope::DiscussionRepository(discussion_id))
            .await?;
        let Some(reply) = reply_by_id(&tx, reply_id).await? else {
            return Ok(None);
        };
        if reply.discussion_id != discussion_id {
            return Ok(None);
        }
        let model = reply_read_model(&tx, reply).await?;
        tx.commit().await?;
        Ok(Some(model))
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "request_discussion_reply_read_model"))]
    pub async fn request_discussion_reply_read_model(
        &self,
        reply: scope_domain::requests::RequestDiscussionReply,
    ) -> Result<ReplyWithActors, PostgresError> {
        let tx = self
            .db
            .begin_read_snapshot(DataScope::DiscussionRepository(&reply.discussion_id))
            .await?;
        let model = reply_read_model(&tx, reply).await?;
        tx.commit().await?;
        Ok(model)
    }
}

async fn reply_read_model(
    tx: &DatabaseTransaction,
    reply: scope_domain::requests::RequestDiscussionReply,
) -> Result<ReplyWithActors, PostgresError> {
    let reply_to = match reply.reply_to_reply_id.as_deref() {
        Some(reply_id) => {
            let target = reply_by_id(tx, reply_id).await?.ok_or_else(|| {
                PostgresError::internal_message(format!(
                    "discussion reply references missing reply {reply_id}"
                ))
            })?;
            if target.discussion_id != reply.discussion_id || target.position >= reply.position {
                return Err(PostgresError::internal_message(format!(
                    "discussion reply {} has an invalid reply target",
                    reply.id
                )));
            }
            Some(RequestDiscussionReplyReferenceReadModel {
                id: target.id,
                position: target.position,
                author: target.author,
                body_markdown: target.body_markdown,
            })
        }
        None => None,
    };
    let model = RequestDiscussionReplyReadModel { reply, reply_to };
    let mut actor_ids = DiscussionActorIds::default();
    reply_actor_ids(&mut actor_ids, &model);
    Ok((model, actor_ids.load(tx).await?))
}

fn reply_actor_ids(actor_ids: &mut DiscussionActorIds, model: &RequestDiscussionReplyReadModel) {
    actor_ids.author(&model.reply.author);
    if let Some(target) = &model.reply_to {
        actor_ids.author(&target.author);
    }
}
