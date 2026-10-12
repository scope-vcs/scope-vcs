use super::{
    GeneratedIdSource, RequestStore, acquire_aggregate_lock, entities,
    request_auto_merge::{
        automatic_event_id, lock_active_intent_for_request, persist_existing_auto_merge_mutation,
    },
    request_revision_rows::latest_revision_for_request,
    request_rows::{request_by_id, save_request_row},
};
use crate::error::PostgresError;
use scope_domain::requests::{
    REQUEST_STACK_MAX_DEPTH, Request, RequestAutoMergeStopReason, RequestBase,
    RequestParentGitFacts, refresh_request_stack, stop_request_auto_merge,
    validate_request_stack_base,
};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseTransaction, EntityTrait, QueryFilter, QueryOrder,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
pub struct RequestParentPublicationFacts {
    pub request_id: String,
    pub revision_id: String,
    pub head_oid: String,
    pub base_version: u64,
    pub parent_contains_pin: bool,
    pub child_contains_pin: bool,
}

#[derive(Clone, Debug)]
pub struct RequestStackChildFacts {
    pub request_id: String,
    pub head_oid: String,
    pub base_version: u64,
    pub parent_contains_pin: bool,
    pub contained_main_oid: Option<String>,
    pub contained_main_descends_from_ancestor: bool,
}

impl RequestStore {
    pub async fn request_stack_children(
        &self,
        parent_id: &str,
    ) -> Result<Vec<Request>, PostgresError> {
        children(self.db.as_ref(), parent_id).await
    }

    pub async fn mark_request_integration_conflict(
        &self,
        request_id: &str,
        actor_user_id: &str,
        expected_head_oid: &str,
        expected_base_version: u64,
        expected_main_oid: &str,
        now_unix: u64,
    ) -> Result<Request, PostgresError> {
        use sea_orm::TransactionTrait;
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let observed = request_by_id(&tx, request_id)
            .await?
            .ok_or_else(|| PostgresError::not_found("request not found"))?;
        acquire_aggregate_lock(&tx, "repository", &observed.repo_id).await?;
        lock_graph(&tx, &observed).await?;
        let (repo, request) =
            super::request_access::lock_request_repository(&tx, request_id, actor_user_id).await?;
        if !super::request_access::request_policy_for_user(&tx, &repo, &request, actor_user_id)
            .await?
            .branch_mutable
        {
            return Err(PostgresError::permission_denied(
                "request branch edit permission required",
            ));
        }
        let main = entities::git_head::Entity::find_by_id(&request.repo_id)
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::conflict("repository has no accepted main"))?;
        if main.head_oid != expected_main_oid {
            return Err(PostgresError::conflict(
                "main changed while computing request integration",
            ));
        }
        let mutation = scope_domain::requests::mark_request_integration_conflict(
            request,
            expected_head_oid,
            expected_base_version,
            now_unix,
        )?;
        let request = persist_refresh(
            &tx,
            mutation,
            RequestAutoMergeStopReason::MergeConflict,
            now_unix,
        )
        .await?;
        let facts = children(&tx, &request.id)
            .await?
            .into_iter()
            .map(|child| RequestStackChildFacts {
                request_id: child.id,
                head_oid: child.head_oid,
                base_version: child.base_version,
                parent_contains_pin: child.stack_status
                    != scope_domain::requests::RequestStackStatus::ParentRewritten,
                contained_main_oid: None,
                contained_main_descends_from_ancestor: false,
            })
            .collect::<Vec<_>>();
        refresh_children(&tx, &request, &facts, now_unix).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(request)
    }
}

pub(super) async fn children<C: ConnectionTrait>(
    conn: &C,
    parent_id: &str,
) -> Result<Vec<Request>, PostgresError> {
    entities::request::Entity::find()
        .filter(entities::request::Column::ParentRequestId.eq(parent_id))
        .filter(entities::request::Column::ClosedAtUnix.is_null())
        .filter(entities::request::Column::MergedAtUnix.is_null())
        .order_by_asc(entities::request::Column::Id)
        .all(conn)
        .await
        .map_err(PostgresError::internal)?
        .into_iter()
        .map(entities::request::Model::try_into_domain)
        .collect()
}

pub(super) async fn ancestry<C: ConnectionTrait>(
    conn: &C,
    base: &RequestBase,
) -> Result<Vec<Request>, PostgresError> {
    let mut next = match base {
        RequestBase::Main { .. } => None,
        RequestBase::Request { request_id, .. } => Some(request_id.clone()),
    };
    let mut result = Vec::new();
    let mut seen = BTreeSet::new();
    while let Some(id) = next {
        if !seen.insert(id.clone()) || result.len() >= REQUEST_STACK_MAX_DEPTH as usize {
            return Err(PostgresError::conflict(
                "request stack is cyclic or exceeds depth 32",
            ));
        }
        let request = request_by_id(conn, &id)
            .await?
            .ok_or_else(|| PostgresError::conflict("request parent is unavailable"))?;
        next = match &request.base {
            RequestBase::Main { .. } => None,
            RequestBase::Request { request_id, .. } => Some(request_id.clone()),
        };
        result.push(request);
    }
    Ok(result)
}

pub(super) async fn validate_start<C: ConnectionTrait>(
    conn: &C,
    input: &mut scope_domain::requests::StartRequestInput,
) -> Result<u32, PostgresError> {
    let ancestors = ancestry(conn, &input.base).await?;
    let revision = match ancestors.first() {
        Some(parent) => latest_revision_for_request(conn, &parent.id).await?,
        None => None,
    };
    let depth = validate_request_stack_base(
        &input.id,
        &input.repo_id,
        &input.view,
        &input.base,
        &ancestors,
        revision.as_ref(),
    )?;
    if let Some(parent) = ancestors.first() {
        input.main_ancestor_oid = parent.main_ancestor_oid.clone();
    }
    Ok(depth)
}

pub(super) async fn validate_publication(
    tx: &DatabaseTransaction,
    request: &Request,
    selected_base: &RequestBase,
    expected_base_version: u64,
    explicit_repin: bool,
    facts: Option<&RequestParentPublicationFacts>,
) -> Result<Option<Request>, PostgresError> {
    if request.base_version != expected_base_version {
        return Err(PostgresError::conflict(
            "request base changed during publication",
        ));
    }
    let RequestBase::Request {
        request_id,
        head_oid,
        ..
    } = selected_base
    else {
        if facts.is_some() {
            return Err(PostgresError::conflict(
                "main publication cannot carry parent facts",
            ));
        }
        return Ok(None);
    };
    let facts =
        facts.ok_or_else(|| PostgresError::conflict("parent publication requires Git evidence"))?;
    let ancestors = ancestry(tx, selected_base).await?;
    for ancestor in &ancestors {
        acquire_aggregate_lock(tx, "request", &ancestor.id).await?;
    }
    let parent = ancestors
        .first()
        .ok_or_else(|| PostgresError::conflict("request parent is unavailable"))?;
    let revision = latest_revision_for_request(tx, &parent.id)
        .await?
        .ok_or_else(|| PostgresError::conflict("parent requires a saved current revision"))?;
    if facts.request_id != *request_id
        || facts.revision_id != revision.id
        || facts.head_oid != parent.head_oid
        || facts.base_version != parent.base_version
        || !facts.parent_contains_pin
        || !facts.child_contains_pin
    {
        return Err(PostgresError::conflict(
            "parent changed or publication does not contain its pin",
        ));
    }
    let current_base = RequestBase::Request {
        request_id: parent.id.clone(),
        revision_id: revision.id.clone(),
        head_oid: parent.head_oid.clone(),
    };
    validate_request_stack_base(
        &request.id,
        &request.repo_id,
        &request.view,
        if explicit_repin {
            selected_base
        } else {
            &current_base
        },
        &ancestors,
        Some(&revision),
    )?;
    if selected_base != &request.base && selected_base != &current_base {
        return Err(PostgresError::conflict(
            "repinning requires the parent's current saved revision",
        ));
    }
    if selected_base.review_base_oid() != head_oid {
        return Err(PostgresError::conflict(
            "parent review base does not match the pin",
        ));
    }
    Ok(Some(parent.clone()))
}

pub(super) async fn refresh_children(
    tx: &DatabaseTransaction,
    parent: &Request,
    facts: &[RequestStackChildFacts],
    now_unix: u64,
) -> Result<(), PostgresError> {
    let direct = children(tx, &parent.id).await?;
    let by_id = facts
        .iter()
        .map(|fact| (fact.request_id.as_str(), fact))
        .collect::<BTreeMap<_, _>>();
    if by_id.len() != facts.len() || by_id.len() != direct.len() {
        return Err(PostgresError::conflict(
            "request children changed during preparation",
        ));
    }
    let mut pending = Vec::new();
    for child in direct {
        let fact = by_id.get(child.id.as_str()).ok_or_else(|| {
            PostgresError::conflict("request children changed during preparation")
        })?;
        if fact.head_oid != child.head_oid || fact.base_version != child.base_version {
            return Err(PostgresError::conflict(
                "request child changed during preparation",
            ));
        }
        pending.push((child, parent.clone(), fact.parent_contains_pin));
    }
    let mut seen = BTreeSet::new();
    while let Some((child, parent, contains_pin)) = pending.pop() {
        if !seen.insert(child.id.clone()) {
            return Err(PostgresError::conflict("request stack contains a cycle"));
        }
        acquire_aggregate_lock(tx, "request", &child.id).await?;
        let mutation = refresh_request_stack(
            child.clone(),
            Some(&parent),
            RequestParentGitFacts {
                parent_contains_pin: contains_pin,
                ancestor_blocked: parent.stack_status.requires_restack(),
                integration_conflict: child.stack_status
                    == scope_domain::requests::RequestStackStatus::IntegrationConflict,
            },
            now_unix,
        )?;
        let updated = persist_refresh(
            tx,
            mutation,
            RequestAutoMergeStopReason::RequestChanged,
            now_unix,
        )
        .await?;
        for descendant in children(tx, &updated.id).await? {
            let contains = descendant.stack_status
                != scope_domain::requests::RequestStackStatus::ParentRewritten;
            pending.push((descendant, updated.clone(), contains));
        }
    }
    Ok(())
}

async fn persist_refresh(
    tx: &DatabaseTransaction,
    mutation: scope_domain::requests::RequestStackRefreshMutation,
    stop_reason: RequestAutoMergeStopReason,
    now_unix: u64,
) -> Result<Request, PostgresError> {
    let mut request = mutation.request;
    if mutation.effects.stop_auto_merge
        && let Some(stored) = lock_active_intent_for_request(tx, &request.id).await?
    {
        let stopped = stop_request_auto_merge(
            &request,
            &stored.intent,
            stop_reason,
            automatic_event_id("stopped", &stored.intent.id),
            now_unix.max(request.updated_at_unix),
        )?;
        persist_existing_auto_merge_mutation(tx, stored.model, &stopped).await?;
        request = stopped.request;
    } else {
        save_request_row(tx, &request).await?;
    }
    if mutation.effects.refresh_queue {
        super::request_attention::reactivate_attention_for_activity(
            tx,
            &request.id,
            "",
            request.activity_version,
            now_unix,
        )
        .await?;
    }
    Ok(request)
}

pub(super) async fn close_children(
    tx: &DatabaseTransaction,
    parent: &Request,
    now_unix: u64,
) -> Result<(), PostgresError> {
    let facts = children(tx, &parent.id)
        .await?
        .into_iter()
        .map(|child| RequestStackChildFacts {
            request_id: child.id,
            head_oid: child.head_oid,
            base_version: child.base_version,
            parent_contains_pin: true,
            contained_main_oid: None,
            contained_main_descends_from_ancestor: false,
        })
        .collect::<Vec<_>>();
    refresh_children(tx, parent, &facts, now_unix).await
}

pub(super) async fn lock_graph(
    tx: &DatabaseTransaction,
    request: &Request,
) -> Result<(), PostgresError> {
    let request = request_by_id(tx, &request.id)
        .await?
        .ok_or_else(|| PostgresError::not_found("request not found"))?;
    let mut ids = ancestry(tx, &request.base)
        .await?
        .into_iter()
        .map(|parent| parent.id)
        .collect::<BTreeSet<_>>();
    let mut pending = vec![request.id.clone()];
    while let Some(id) = pending.pop() {
        if !ids.insert(id.clone()) {
            return Err(PostgresError::conflict("request stack contains a cycle"));
        }
        pending.extend(children(tx, &id).await?.into_iter().map(|child| child.id));
    }
    for id in ids {
        acquire_aggregate_lock(tx, "request", &id).await?;
    }
    Ok(())
}

pub(super) async fn retarget_children(
    tx: &DatabaseTransaction,
    parent: &Request,
    facts: &[RequestStackChildFacts],
    now_unix: u64,
    generated_ids: &dyn GeneratedIdSource,
) -> Result<(), PostgresError> {
    let direct = children(tx, &parent.id).await?;
    let by_id = facts
        .iter()
        .map(|fact| (fact.request_id.as_str(), fact))
        .collect::<BTreeMap<_, _>>();
    if by_id.len() != facts.len() || by_id.len() != direct.len() {
        return Err(PostgresError::conflict(
            "request children changed during preparation",
        ));
    }
    for child in direct {
        let fact = by_id.get(child.id.as_str()).ok_or_else(|| {
            PostgresError::conflict("request children changed during preparation")
        })?;
        if fact.head_oid != child.head_oid || fact.base_version != child.base_version {
            return Err(PostgresError::conflict(
                "request child changed during preparation",
            ));
        }
        if !fact.parent_contains_pin {
            continue;
        }
        let mutation = scope_domain::requests::retarget_request_after_parent_merge(
            child,
            parent,
            scope_domain::requests::RetargetRequestAfterParentMergeInput {
                event_id: super::generated_ids::generate_id(
                    generated_ids,
                    super::GeneratedIdKind::RequestMergedEvent,
                )?,
                now_unix,
                parent_contains_pin: fact.parent_contains_pin,
                contained_main_oid: fact.contained_main_oid.clone(),
                contained_main_descends_from_ancestor: fact.contained_main_descends_from_ancestor,
            },
        )?;
        save_request_row(tx, &mutation.request).await?;
        super::request_rows::insert_request_event_row(tx, &mutation.event).await?;
        if mutation.effects.rebind_check_context {
            tx.execute_raw(sea_orm::Statement::from_sql_and_values(sea_orm::DatabaseBackend::Postgres,
                "UPDATE scope_request_check_evaluations SET base_version = $2 WHERE id = (SELECT current_check_evaluation_id FROM scope_requests WHERE id = $1)",
                [mutation.request.id.clone().into(), super::integer_columns::u64_to_i64(mutation.request.base_version, "request base version")?.into()])).await.map_err(PostgresError::internal)?;
        }
        if mutation.effects.refresh_queue {
            super::request_attention::reactivate_attention_for_activity(
                tx,
                &mutation.request.id,
                "",
                mutation.request.activity_version,
                now_unix,
            )
            .await?;
        }
        tx.execute_raw(sea_orm::Statement::from_sql_and_values(sea_orm::DatabaseBackend::Postgres,
            "UPDATE scope_request_auto_merge_intents SET next_attempt_at_unix = $2 WHERE request_id = $1 AND status = 'Active'",
            [mutation.request.id.clone().into(), super::integer_columns::u64_to_i64(now_unix, "auto-merge wake time")?.into()])).await.map_err(PostgresError::internal)?;
        let descendants = children(tx, &mutation.request.id)
            .await?
            .into_iter()
            .map(|child| RequestStackChildFacts {
                request_id: child.id,
                head_oid: child.head_oid,
                base_version: child.base_version,
                parent_contains_pin: child.stack_status
                    != scope_domain::requests::RequestStackStatus::ParentRewritten,
                contained_main_oid: None,
                contained_main_descends_from_ancestor: false,
            })
            .collect::<Vec<_>>();
        refresh_children(tx, &mutation.request, &descendants, now_unix).await?;
    }
    let remaining = children(tx, &parent.id).await?;
    let remaining_facts = remaining
        .iter()
        .filter_map(|child| by_id.get(child.id.as_str()).map(|fact| (*fact).clone()))
        .collect::<Vec<_>>();
    refresh_children(tx, parent, &remaining_facts, now_unix).await
}

#[cfg(test)]
mod tests;
