use crate::{
    db::{entities, integer_columns},
    error::PostgresError,
};
use scope_domain::{
    repository::access::RepositoryAccess,
    requests::{
        REQUEST_LIST_MAX_PAGE_SIZE, RequestActorRole, RequestBase, RequestChangePlacement,
        RequestChangeSubject, RequestListPredicate, RequestStackStatus, RequestState,
        RequestViewer, request_change_visible, request_list_predicate,
    },
    views::{ViewId, Views},
};
use sea_orm::{
    ColumnTrait, Condition, ConnectionTrait, EntityTrait, ExprTrait, FromQueryResult, QueryFilter,
    QueryOrder, QuerySelect,
    sea_query::{Expr, Query},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestListRow {
    pub id: String,
    pub name: String,
    pub title: String,
    pub author_user_id: Option<String>,
    pub author_role: RequestActorRole,
    pub view: ViewId,
    pub head_oid: String,
    pub base: RequestBase,
    pub base_version: u64,
    pub main_ancestor_oid: String,
    pub stack_status: RequestStackStatus,
    pub stack_depth: u32,
    pub state: RequestState,
    pub submitted_at_unix: Option<u64>,
    pub closed_at_unix: Option<u64>,
    pub merged_at_unix: Option<u64>,
    pub updated_at_unix: u64,
    pub activity_version: u64,
    pub has_pushed_head: bool,
    pub awaits_change: bool,
    pub viewer_reads_change: bool,
}

#[derive(Clone, Copy)]
pub(in crate::db) struct RequestListReader<'a> {
    pub(in crate::db) access: &'a RepositoryAccess,
    pub(in crate::db) user_id: Option<&'a str>,
    pub(in crate::db) views: &'a Views,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestListPageQuery<'a> {
    pub repo_id: &'a str,
    pub viewer_user_id: Option<&'a str>,
    pub access: RepositoryAccess,
    pub views: &'a Views,
    pub after_id: Option<&'a str>,
    pub name: Option<&'a str>,
    pub limit: u64,
}

#[derive(Clone, Debug, FromQueryResult)]
pub(in crate::db) struct RequestListModel {
    pub(in crate::db) id: String,
    name: String,
    title: String,
    author_user_id: Option<String>,
    author_role: String,
    audience: String,
    head_oid: String,
    base_kind: String,
    review_base_oid: String,
    parent_request_id: Option<String>,
    parent_revision_id: Option<String>,
    parent_head_oid: Option<String>,
    base_version: i64,
    main_ancestor_oid: String,
    stack_status: String,
    stack_depth: i32,
    submitted_at_unix: Option<i64>,
    closed_at_unix: Option<i64>,
    merged_at_unix: Option<i64>,
    updated_at_unix: i64,
    activity_version: i64,
    has_pushed_head: bool,
    awaits_change: bool,
    pub(in crate::db) viewer_is_invitee: bool,
}

impl RequestListModel {
    pub(in crate::db) fn try_into_read_model(
        self,
        reader: RequestListReader<'_>,
    ) -> Result<RequestListRow, PostgresError> {
        let submitted_at_unix = self
            .submitted_at_unix
            .map(|value| integer_columns::i64_to_u64(value, "request submission time"))
            .transpose()?;
        let closed_at_unix = self
            .closed_at_unix
            .map(|value| integer_columns::i64_to_u64(value, "request close time"))
            .transpose()?;
        let merged_at_unix = self
            .merged_at_unix
            .map(|value| integer_columns::i64_to_u64(value, "request merge time"))
            .transpose()?;
        let view = ViewId::parse(&self.audience).map_err(PostgresError::internal)?;
        let viewer_reads_change = request_change_visible(
            RequestChangeSubject {
                view: &view,
                author_user_id: self.author_user_id.as_deref(),
                submitted: submitted_at_unix.is_some(),
            },
            &RequestViewer::new(
                reader.access.clone(),
                reader.user_id,
                self.viewer_is_invitee,
            ),
            reader.views,
        );
        let change = if viewer_reads_change {
            RequestChangePlacement {
                head_oid: self.head_oid,
                base: entities::request::BaseColumns {
                    kind: self.base_kind,
                    review_base_oid: self.review_base_oid,
                    parent_request_id: self.parent_request_id,
                    parent_revision_id: self.parent_revision_id,
                    parent_head_oid: self.parent_head_oid,
                }
                .try_into_domain()?,
                base_version: integer_columns::i64_to_u64(
                    self.base_version,
                    "request base version",
                )?,
                stack_status: entities::decode_enum(self.stack_status)?,
                stack_depth: self
                    .stack_depth
                    .try_into()
                    .map_err(PostgresError::internal)?,
            }
        } else {
            RequestChangePlacement::hidden(&self.main_ancestor_oid)
        };
        Ok(RequestListRow {
            id: self.id,
            name: self.name,
            title: self.title,
            author_user_id: self.author_user_id,
            author_role: entities::decode_enum(self.author_role)?,
            view,
            head_oid: change.head_oid,
            base: change.base,
            base_version: change.base_version,
            main_ancestor_oid: self.main_ancestor_oid,
            stack_status: change.stack_status,
            stack_depth: change.stack_depth,
            state: RequestState::from_timestamps(merged_at_unix, closed_at_unix, submitted_at_unix),
            submitted_at_unix,
            closed_at_unix,
            merged_at_unix,
            updated_at_unix: integer_columns::i64_to_u64(
                self.updated_at_unix,
                "request update time",
            )?,
            activity_version: integer_columns::i64_to_u64(
                self.activity_version,
                "request activity version",
            )?,
            has_pushed_head: self.has_pushed_head,
            awaits_change: self.awaits_change,
            viewer_reads_change,
        })
    }
}

pub(in crate::db) async fn request_list_page<C>(
    conn: &C,
    input: RequestListPageQuery<'_>,
) -> Result<Vec<RequestListRow>, PostgresError>
where
    C: ConnectionTrait,
{
    request_list_rows(
        conn,
        request_list_select(&input)?,
        RequestListReader {
            access: &input.access,
            user_id: input.viewer_user_id,
            views: input.views,
        },
    )
    .await
}

pub(in crate::db) async fn request_list_rows<C>(
    conn: &C,
    query: sea_orm::Select<entities::request::Entity>,
    reader: RequestListReader<'_>,
) -> Result<Vec<RequestListRow>, PostgresError>
where
    C: ConnectionTrait,
{
    query
        .into_model::<RequestListModel>()
        .all(conn)
        .await?
        .into_iter()
        .map(|row| row.try_into_read_model(reader))
        .collect()
}

fn request_list_select(
    input: &RequestListPageQuery<'_>,
) -> Result<sea_orm::Select<entities::request::Entity>, PostgresError> {
    let mut query = request_list_projection(input.viewer_user_id)
        .filter(entities::request::Column::RepoId.eq(input.repo_id))
        .order_by_asc(entities::request::Column::Id)
        .limit(std::cmp::min(
            input.limit,
            (REQUEST_LIST_MAX_PAGE_SIZE + 1) as u64,
        ));
    if let Some(after_id) = input.after_id {
        query = query.filter(entities::request::Column::Id.gt(after_id));
    }
    if let Some(name) = input.name {
        query = query.filter(entities::request::Column::Name.eq(name));
    }
    query = query.filter(request_list_condition(&request_list_predicate(
        input.access.clone(),
        input.viewer_user_id,
        input.views,
    ))?);
    Ok(query)
}

pub(in crate::db) fn request_list_condition(
    predicate: &RequestListPredicate<'_>,
) -> Result<Condition, PostgresError> {
    match predicate {
        RequestListPredicate::All(predicates) => predicates
            .iter()
            .try_fold(Condition::all(), |condition, predicate| {
                Ok(condition.add(request_list_condition(predicate)?))
            }),
        RequestListPredicate::Any(predicates) => predicates
            .iter()
            .try_fold(Condition::any(), |condition, predicate| {
                Ok(condition.add(request_list_condition(predicate)?))
            }),
        RequestListPredicate::View(view) => {
            Ok(Condition::all()
                .add(entities::request::Column::Audience.eq(view.as_str().to_string())))
        }
        RequestListPredicate::Submitted => {
            Ok(Condition::all().add(entities::request::Column::SubmittedAtUnix.is_not_null()))
        }
        RequestListPredicate::Asked => {
            Ok(Condition::all().add(entities::request::Column::GithubIssueNumber.is_not_null()))
        }
        RequestListPredicate::Closed => {
            Ok(Condition::all().add(entities::request::Column::ClosedAtUnix.is_not_null()))
        }
        RequestListPredicate::Author(viewer_user_id) => {
            Ok(Condition::all().add(entities::request::Column::AuthorUserId.eq(*viewer_user_id)))
        }
        RequestListPredicate::Invitee(viewer_user_id) => {
            Ok(Condition::all().add(Expr::exists(invitation_of(viewer_user_id))))
        }
    }
}

fn invitation_of(viewer_user_id: &str) -> sea_orm::sea_query::SelectStatement {
    Query::select()
        .expr(Expr::val(1))
        .from(entities::request_invitee::Entity)
        .and_where(
            Expr::col((
                entities::request_invitee::Entity,
                entities::request_invitee::Column::RequestId,
            ))
            .equals((entities::request::Entity, entities::request::Column::Id)),
        )
        .and_where(entities::request_invitee::Column::UserId.eq(viewer_user_id))
        .to_owned()
}

pub(in crate::db) fn request_list_projection(
    viewer_user_id: Option<&str>,
) -> sea_orm::Select<entities::request::Entity> {
    entities::request::Entity::find()
        .select_only()
        .column(entities::request::Column::Id)
        .column(entities::request::Column::Name)
        .column(entities::request::Column::Title)
        .column(entities::request::Column::AuthorUserId)
        .column(entities::request::Column::AuthorRole)
        .column(entities::request::Column::Audience)
        .column(entities::request::Column::HeadOid)
        .column(entities::request::Column::BaseKind)
        .column(entities::request::Column::ReviewBaseOid)
        .column(entities::request::Column::ParentRequestId)
        .column(entities::request::Column::ParentRevisionId)
        .column(entities::request::Column::ParentHeadOid)
        .column(entities::request::Column::BaseVersion)
        .column(entities::request::Column::MainAncestorOid)
        .column(entities::request::Column::StackStatus)
        .column(entities::request::Column::StackDepth)
        .column(entities::request::Column::SubmittedAtUnix)
        .column(entities::request::Column::ClosedAtUnix)
        .column(entities::request::Column::MergedAtUnix)
        .column(entities::request::Column::UpdatedAtUnix)
        .column(entities::request::Column::ActivityVersion)
        .expr_as(
            Expr::col(entities::request::Column::PushedAtUnix).is_not_null(),
            "has_pushed_head",
        )
        .expr_as(
            Expr::col(entities::request::Column::GithubIssueNumber)
                .is_not_null()
                .and(Expr::col(entities::request::Column::PushedAtUnix).is_null()),
            "awaits_change",
        )
        .expr_as(
            match viewer_user_id {
                Some(viewer_user_id) => Expr::exists(invitation_of(viewer_user_id)),
                None => Expr::val(false),
            },
            "viewer_is_invitee",
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{DatabaseBackend, QueryTrait};

    #[test]
    fn request_list_does_not_fetch_unbounded_payloads() {
        let query = request_list_select(&RequestListPageQuery {
            repo_id: "repo-1",
            viewer_user_id: Some("viewer-1"),
            access: RepositoryAccess::public(),
            views: &Views::builtin(),
            after_id: None,
            name: None,
            limit: 1,
        })
        .unwrap();
        let sql = query.build(DatabaseBackend::Postgres).to_string();
        let projection = sql.split(" FROM ").next().unwrap();

        assert!(!projection.contains("description_markdown"));
        assert_eq!(projection.matches("\"pushed_at_unix\"").count(), 2);
        assert!(projection.contains("pushed_at_unix\" IS NOT NULL"));
    }
}
