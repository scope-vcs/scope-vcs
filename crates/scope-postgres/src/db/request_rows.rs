use super::entities;
use super::integer_columns;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, PaginatorTrait,
    QueryFilter, QueryOrder, QuerySelect, sea_query::Expr,
};
use {
    crate::error::PostgresError,
    scope_domain::requests::{Request, RequestActorRole, RequestEvent},
    scope_domain::views::ViewId,
    std::collections::BTreeMap,
};

mod list;
pub(super) use list::{
    RequestListModel, RequestListReader, request_list_condition, request_list_page,
    request_list_projection, request_list_rows,
};
pub use list::{RequestListPageQuery, RequestListRow};

pub(super) async fn request_by_id<C>(
    conn: &C,
    request_id: &str,
) -> Result<Option<Request>, PostgresError>
where
    C: ConnectionTrait,
{
    entities::request::Entity::find_by_id(request_id.to_string())
        .one(conn)
        .await?
        .map(entities::request::Model::try_into_domain)
        .transpose()
}

pub(super) async fn request_by_name<C>(
    conn: &C,
    repo_id: &str,
    request_name: &str,
) -> Result<Option<Request>, PostgresError>
where
    C: ConnectionTrait,
{
    entities::request::Entity::find()
        .filter(entities::request::Column::RepoId.eq(repo_id.to_string()))
        .filter(entities::request::Column::Name.eq(request_name.to_string()))
        .one(conn)
        .await?
        .map(entities::request::Model::try_into_domain)
        .transpose()
}

pub(super) async fn request_by_github_issue_number<C>(
    conn: &C,
    repo_id: &str,
    github_issue_number: u64,
) -> Result<Option<Request>, PostgresError>
where
    C: ConnectionTrait,
{
    entities::request::Entity::find()
        .filter(entities::request::Column::RepoId.eq(repo_id))
        .filter(
            entities::request::Column::GithubIssueNumber.eq(integer_columns::u64_to_i64(
                github_issue_number,
                "GitHub issue number",
            )?),
        )
        .one(conn)
        .await?
        .map(entities::request::Model::try_into_domain)
        .transpose()
}

pub(super) async fn requests_by_repo_id<C>(
    conn: &C,
    repo_id: &str,
) -> Result<Vec<Request>, PostgresError>
where
    C: ConnectionTrait,
{
    entities::request::Entity::find()
        .filter(entities::request::Column::RepoId.eq(repo_id.to_string()))
        .order_by_asc(entities::request::Column::Id)
        .all(conn)
        .await?
        .into_iter()
        .map(entities::request::Model::try_into_domain)
        .collect()
}

pub(super) async fn public_draft_count<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
    author_user_id: &str,
) -> Result<usize, PostgresError> {
    let count = entities::request::Entity::find()
        .filter(entities::request::Column::RepoId.eq(repo_id))
        .filter(entities::request::Column::AuthorUserId.eq(author_user_id))
        .filter(
            entities::request::Column::AuthorRole
                .eq(entities::encode_enum(RequestActorRole::Public)?),
        )
        .filter(entities::request::Column::SubmittedAtUnix.is_null())
        .filter(entities::request::Column::ClosedAtUnix.is_null())
        .filter(entities::request::Column::MergedAtUnix.is_null())
        .count(conn)
        .await?;
    usize::try_from(count).map_err(PostgresError::internal)
}

pub(super) async fn open_request_counts_by_view<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
) -> Result<BTreeMap<ViewId, usize>, PostgresError> {
    entities::request::Entity::find()
        .select_only()
        .column(entities::request::Column::Audience)
        .column_as(entities::request::Column::Id.count(), "open_requests")
        .filter(entities::request::Column::RepoId.eq(repo_id))
        .filter(entities::request::Column::ClosedAtUnix.is_null())
        .filter(entities::request::Column::MergedAtUnix.is_null())
        .group_by(entities::request::Column::Audience)
        .into_tuple::<(String, i64)>()
        .all(conn)
        .await?
        .into_iter()
        .map(|(view, count)| {
            Ok((
                ViewId::parse(&view).map_err(PostgresError::internal)?,
                usize::try_from(count).map_err(PostgresError::internal)?,
            ))
        })
        .collect()
}

pub(super) async fn request_events_by_request_id<C>(
    conn: &C,
    request_id: &str,
) -> Result<Vec<RequestEvent>, PostgresError>
where
    C: ConnectionTrait,
{
    entities::request_event::Entity::find()
        .filter(entities::request_event::Column::RequestId.eq(request_id.to_string()))
        .order_by_asc(entities::request_event::Column::CreatedAtUnix)
        .order_by_asc(entities::request_event::Column::Position)
        .all(conn)
        .await?
        .into_iter()
        .map(entities::request_event::Model::try_into_domain)
        .collect()
}

pub(super) async fn request_events_after_position<C>(
    conn: &C,
    request_id: &str,
    after_position: u64,
    limit: u64,
) -> Result<Vec<RequestEvent>, PostgresError>
where
    C: ConnectionTrait,
{
    entities::request_event::Entity::find()
        .filter(entities::request_event::Column::RequestId.eq(request_id))
        .filter(
            entities::request_event::Column::Position
                .gt(i64::try_from(after_position).map_err(PostgresError::internal)?),
        )
        .order_by_asc(entities::request_event::Column::Position)
        .limit(limit)
        .all(conn)
        .await?
        .into_iter()
        .map(entities::request_event::Model::try_into_domain)
        .collect()
}

pub(super) async fn latest_request_events<C>(
    conn: &C,
    request_id: &str,
    limit: u64,
) -> Result<Vec<RequestEvent>, PostgresError>
where
    C: ConnectionTrait,
{
    let mut events = entities::request_event::Entity::find()
        .filter(entities::request_event::Column::RequestId.eq(request_id))
        .order_by_desc(entities::request_event::Column::Position)
        .limit(limit)
        .all(conn)
        .await?
        .into_iter()
        .map(entities::request_event::Model::try_into_domain)
        .collect::<Result<Vec<_>, _>>()?;
    events.reverse();
    Ok(events)
}

pub(super) async fn request_event_by_id<C>(
    conn: &C,
    event_id: &str,
) -> Result<Option<RequestEvent>, PostgresError>
where
    C: ConnectionTrait,
{
    entities::request_event::Entity::find_by_id(event_id.to_string())
        .one(conn)
        .await?
        .map(entities::request_event::Model::try_into_domain)
        .transpose()
}

pub(super) async fn insert_request_row<C>(conn: &C, request: &Request) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    entities::request::Model::from_domain(request)?
        .into_active_model()
        .insert(conn)
        .await?;
    Ok(())
}

pub(super) async fn delete_request_rows<C>(conn: &C, request_id: &str) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    entities::request_invitee::Entity::delete_many()
        .filter(entities::request_invitee::Column::RequestId.eq(request_id.to_string()))
        .exec(conn)
        .await?;
    entities::request_event::Entity::delete_many()
        .filter(entities::request_event::Column::RequestId.eq(request_id.to_string()))
        .exec(conn)
        .await?;
    entities::request::Entity::delete_by_id(request_id.to_string())
        .exec(conn)
        .await?;
    Ok(())
}

pub(super) async fn save_request_row<C>(conn: &C, request: &Request) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    let row = entities::request::Model::from_domain(request)?;
    let result = entities::request::Entity::update_many()
        .filter(entities::request::Column::Id.eq(row.id))
        .col_expr(entities::request::Column::Title, Expr::value(row.title))
        .col_expr(
            entities::request::Column::DescriptionMarkdown,
            Expr::value(row.description_markdown),
        )
        .col_expr(
            entities::request::Column::MainAncestorOid,
            Expr::value(row.main_ancestor_oid),
        )
        .col_expr(
            entities::request::Column::BaseKind,
            Expr::value(row.base_kind),
        )
        .col_expr(
            entities::request::Column::ReviewBaseOid,
            Expr::value(row.review_base_oid),
        )
        .col_expr(
            entities::request::Column::ParentRequestId,
            Expr::value(row.parent_request_id),
        )
        .col_expr(
            entities::request::Column::ParentRevisionId,
            Expr::value(row.parent_revision_id),
        )
        .col_expr(
            entities::request::Column::ParentHeadOid,
            Expr::value(row.parent_head_oid),
        )
        .col_expr(
            entities::request::Column::StackStatus,
            Expr::value(row.stack_status),
        )
        .col_expr(
            entities::request::Column::StackDepth,
            Expr::value(row.stack_depth),
        )
        .col_expr(
            entities::request::Column::BaseVersion,
            Expr::value(row.base_version),
        )
        .col_expr(
            entities::request::Column::HeadOid,
            Expr::value(row.head_oid),
        )
        .col_expr(
            entities::request::Column::PushedAtUnix,
            Expr::value(row.pushed_at_unix),
        )
        .col_expr(
            entities::request::Column::ActivityVersion,
            Expr::value(row.activity_version),
        )
        .col_expr(
            entities::request::Column::SubmittedAtUnix,
            Expr::value(row.submitted_at_unix),
        )
        .col_expr(
            entities::request::Column::ClosedAtUnix,
            Expr::value(row.closed_at_unix),
        )
        .col_expr(
            entities::request::Column::ClosedByUserId,
            Expr::value(row.closed_by_user_id),
        )
        .col_expr(
            entities::request::Column::CloseReason,
            Expr::value(row.close_reason),
        )
        .col_expr(
            entities::request::Column::CloseNamedRequestId,
            Expr::value(row.close_named_request_id),
        )
        .col_expr(
            entities::request::Column::MergedAtUnix,
            Expr::value(row.merged_at_unix),
        )
        .col_expr(
            entities::request::Column::MergedByUserId,
            Expr::value(row.merged_by_user_id),
        )
        .col_expr(
            entities::request::Column::MergedHeadOid,
            Expr::value(row.merged_head_oid),
        )
        .col_expr(
            entities::request::Column::MergedMainOid,
            Expr::value(row.merged_main_oid),
        )
        .col_expr(
            entities::request::Column::UpdatedAtUnix,
            Expr::value(row.updated_at_unix),
        )
        .exec(conn)
        .await?;
    if result.rows_affected == 0 {
        return Err(PostgresError::internal_message(
            "request row missing during update",
        ));
    }
    Ok(())
}

pub(super) async fn save_request_branch_and_author<C>(
    conn: &C,
    request: &Request,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    let row = entities::request::Model::from_domain(request)?;
    entities::request::Entity::update_many()
        .filter(entities::request::Column::Id.eq(row.id))
        .col_expr(entities::request::Column::Name, Expr::value(row.name))
        .col_expr(
            entities::request::Column::AuthorUserId,
            Expr::value(row.author_user_id),
        )
        .col_expr(
            entities::request::Column::AuthorRole,
            Expr::value(row.author_role),
        )
        .exec(conn)
        .await?;
    save_request_row(conn, request).await
}

pub(super) async fn insert_request_event_row<C>(
    conn: &C,
    event: &RequestEvent,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    entities::request_event::Model::from_domain(event)?
        .into_active_model()
        .insert(conn)
        .await?;
    Ok(())
}
