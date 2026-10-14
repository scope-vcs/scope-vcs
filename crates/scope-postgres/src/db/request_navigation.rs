use super::{RequestListRow, RequestStore, begin_metadata_read_snapshot, entities, request_rows};
use crate::error::PostgresError;
use scope_domain::{
    repository::access::RepositoryAccessContext,
    requests::{
        REQUEST_LIST_MAX_PAGE_SIZE, REQUEST_STACK_MAX_ANCESTORS, RequestBase,
        RequestStackNavigation, RequestStackNode, RequestStackParent, request_list_predicate,
        request_stack_navigation,
    },
};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseBackend, FromQueryResult, QueryFilter, QueryOrder,
    QuerySelect, Statement,
};
use std::collections::{BTreeMap, BTreeSet};

pub struct RequestChildrenPage {
    pub requests: Vec<RequestStackNode>,
    pub next_cursor: Option<String>,
}

impl RequestStore {
    pub async fn request_stack_parents(
        &self,
        repository: &RepositoryAccessContext,
        viewer_user_id: Option<&str>,
        rows: &[RequestListRow],
    ) -> Result<BTreeMap<String, RequestStackParent>, PostgresError> {
        let ids = rows
            .iter()
            .filter_map(|row| row.base.parent_request_id().map(str::to_string))
            .collect::<BTreeSet<_>>();
        let tx = begin_metadata_read_snapshot(self.db.as_ref()).await?;
        let parents = authorized_nodes(&tx, repository, viewer_user_id, &ids).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(rows
            .iter()
            .filter_map(|row| {
                let id = row.base.parent_request_id()?;
                let parent = parents
                    .get(id)
                    .filter(|(_, _, view)| view == &row.view)
                    .map_or(RequestStackParent::Unavailable, |(node, _, _)| {
                        RequestStackParent::Available {
                            request: node.clone(),
                        }
                    });
                Some((row.id.clone(), parent))
            })
            .collect())
    }
}

type AuthorizedNodes =
    BTreeMap<String, (RequestStackNode, RequestBase, scope_domain::views::ViewId)>;

async fn authorized_nodes<C: ConnectionTrait>(
    conn: &C,
    repository: &RepositoryAccessContext,
    viewer_user_id: Option<&str>,
    ids: &BTreeSet<String>,
) -> Result<AuthorizedNodes, PostgresError> {
    if ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let query = request_rows::request_list_projection()
        .filter(entities::request::Column::RepoId.eq(&repository.record.id))
        .filter(entities::request::Column::Id.is_in(ids.iter().cloned()))
        .filter(request_rows::request_list_condition(
            &request_list_predicate(repository.access.clone(), viewer_user_id, &repository.views),
        )?);
    let rows = request_rows::request_list_rows(conn, query).await?;
    nodes_for_rows(conn, rows).await
}

#[derive(FromQueryResult)]
struct AncestorId {
    id: String,
}

#[derive(FromQueryResult)]
struct CurrentRevision {
    request_id: String,
    id: String,
}

async fn nodes_for_rows<C: ConnectionTrait>(
    conn: &C,
    rows: Vec<RequestListRow>,
) -> Result<AuthorizedNodes, PostgresError> {
    if rows.is_empty() {
        return Ok(BTreeMap::new());
    }
    let placeholders = (1..=rows.len())
        .map(|index| format!("${index}"))
        .collect::<Vec<_>>()
        .join(",");
    let revisions = CurrentRevision::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        format!("SELECT DISTINCT ON (request_id) request_id, id FROM scope_request_revisions WHERE request_id IN ({placeholders}) ORDER BY request_id, position DESC, id DESC"),
        rows.iter().map(|row| row.id.clone().into()).collect::<Vec<sea_orm::Value>>(),
    )).all(conn).await.map_err(PostgresError::internal)?.into_iter().map(|row| (row.request_id, row.id)).collect::<BTreeMap<_, _>>();
    Ok(rows
        .into_iter()
        .map(|row| {
            let node = RequestStackNode {
                current_revision_id: revisions.get(&row.id).cloned(),
                id: row.id.clone(),
                name: row.name,
                title: row.title,
                state: row.state,
                stack_status: row.stack_status,
                stack_depth: row.stack_depth,
                head_oid: row.head_oid,
            };
            (row.id, (node, row.base, row.view))
        })
        .collect())
}

pub(super) async fn navigation<C: ConnectionTrait>(
    conn: &C,
    repository: &RepositoryAccessContext,
    viewer_user_id: Option<&str>,
    request: &scope_domain::requests::Request,
    children_after_id: Option<&str>,
    children_limit: usize,
) -> Result<(RequestStackNavigation, RequestChildrenPage), PostgresError> {
    let ancestors = match request.base.parent_request_id() {
        Some(parent_id) => {
            let ids = AncestorId::find_by_statement(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "WITH RECURSIVE ancestors AS (SELECT id, parent_request_id, 1 AS depth, ARRAY[id] AS visited FROM scope_requests WHERE id = $1 AND repo_id = $2 AND audience = $3 UNION ALL SELECT parent.id, parent.parent_request_id, ancestors.depth + 1, ancestors.visited || parent.id FROM scope_requests parent JOIN ancestors ON parent.id = ancestors.parent_request_id WHERE parent.repo_id = $2 AND parent.audience = $3 AND ancestors.depth < $4 AND NOT parent.id = ANY(ancestors.visited)) SELECT id FROM ancestors",
                vec![parent_id.into(), repository.record.id.clone().into(), request.view.as_str().into(), (REQUEST_STACK_MAX_ANCESTORS as i32).into()],
            )).all(conn).await.map_err(PostgresError::internal)?.into_iter().map(|row| row.id).collect();
            authorized_nodes(conn, repository, viewer_user_id, &ids)
                .await?
                .into_iter()
                .map(|(id, (node, base, _))| (id, (node, base)))
                .collect()
        }
        None => BTreeMap::new(),
    };
    let navigation = request_stack_navigation(&request.id, &request.base, &ancestors);
    let children = children_page(
        conn,
        repository,
        viewer_user_id,
        request,
        children_after_id,
        children_limit,
    )
    .await?;
    Ok((navigation, children))
}

async fn children_page<C: ConnectionTrait>(
    conn: &C,
    repository: &RepositoryAccessContext,
    viewer_user_id: Option<&str>,
    request: &scope_domain::requests::Request,
    after_id: Option<&str>,
    limit: usize,
) -> Result<RequestChildrenPage, PostgresError> {
    let limit = limit.clamp(1, REQUEST_LIST_MAX_PAGE_SIZE);
    let mut query = request_rows::request_list_projection()
        .filter(entities::request::Column::RepoId.eq(&repository.record.id))
        .filter(entities::request::Column::Audience.eq(request.view.as_str()))
        .filter(entities::request::Column::ParentRequestId.eq(&request.id))
        .filter(request_rows::request_list_condition(
            &request_list_predicate(repository.access.clone(), viewer_user_id, &repository.views),
        )?)
        .order_by_asc(entities::request::Column::Id)
        .limit((limit + 1) as u64);
    if let Some(id) = after_id {
        query = query.filter(entities::request::Column::Id.gt(id));
    }
    let mut rows = request_rows::request_list_rows(conn, query).await?;
    let has_more = rows.len() > limit;
    rows.truncate(limit);
    let next_cursor = has_more
        .then(|| rows.last().map(|row| format!("v1:{}", row.id)))
        .flatten();
    let nodes = nodes_for_rows(conn, rows).await?;
    Ok(RequestChildrenPage {
        requests: nodes.into_values().map(|(node, _, _)| node).collect(),
        next_cursor,
    })
}
