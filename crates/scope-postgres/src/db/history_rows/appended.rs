use super::{
    file_change_from_row, logical_commit_from_row, visibility_change_from_row,
    visibility_change_set_from_row,
};
use crate::db::entities;
use crate::db::integer_columns::usize_to_i64;
use crate::error::PostgresError;
use scope_domain::{
    projection::{FileChange, LogicalCommit},
    visibility_changes::{VisibilityChange, VisibilityChangeSet},
};
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, sea_query::Query,
};
use std::collections::BTreeMap;

pub(crate) struct AppendedRepositoryHistory {
    pub commits: Vec<LogicalCommit>,
    pub visibility_change_sets: Vec<VisibilityChangeSet>,
}

pub(crate) async fn history_position_matches<C>(
    conn: &C,
    repo_id: &str,
    position: &entities::projection_read_model::FoldPosition,
) -> Result<bool, PostgresError>
where
    C: ConnectionTrait,
{
    let last_commit_id = match position.commits.checked_sub(1) {
        Some(ordinal) => entities::logical_commit::Entity::find()
            .filter(entities::logical_commit::Column::RepoId.eq(repo_id.to_string()))
            .filter(
                entities::logical_commit::Column::Ordinal
                    .eq(usize_to_i64(ordinal, "commit ordinal")?),
            )
            .one(conn)
            .await
            .map_err(PostgresError::internal)?
            .map(|row| row.id),
        None => None,
    };
    let last_change_set_id = match position.change_sets.checked_sub(1) {
        Some(ordinal) => entities::visibility_change_set::Entity::find()
            .filter(entities::visibility_change_set::Column::RepoId.eq(repo_id.to_string()))
            .filter(
                entities::visibility_change_set::Column::Ordinal
                    .eq(usize_to_i64(ordinal, "change set ordinal")?),
            )
            .one(conn)
            .await
            .map_err(PostgresError::internal)?
            .map(|row| row.id),
        None => None,
    };
    Ok(last_commit_id.is_some() == (position.commits > 0)
        && last_commit_id == position.last_commit_id
        && last_change_set_id.is_some() == (position.change_sets > 0)
        && last_change_set_id == position.last_change_set_id)
}

pub(crate) async fn load_repository_history_after<C>(
    conn: &C,
    repo_id: &str,
    commits_from: usize,
    change_sets_from: usize,
) -> Result<AppendedRepositoryHistory, PostgresError>
where
    C: ConnectionTrait,
{
    let commit_rows = entities::logical_commit::Entity::find()
        .filter(entities::logical_commit::Column::RepoId.eq(repo_id.to_string()))
        .filter(
            entities::logical_commit::Column::Ordinal
                .gte(usize_to_i64(commits_from, "commit ordinal")?),
        )
        .order_by_asc(entities::logical_commit::Column::Ordinal)
        .all(conn)
        .await
        .map_err(PostgresError::internal)?;
    let mut changes_by_commit = BTreeMap::<String, Vec<FileChange>>::new();
    if !commit_rows.is_empty() {
        for row in entities::file_change::Entity::find()
            .filter(entities::file_change::Column::RepoId.eq(repo_id.to_string()))
            .filter(
                entities::file_change::Column::CommitId.in_subquery(
                    Query::select()
                        .column(entities::logical_commit::Column::Id)
                        .from(entities::logical_commit::Entity)
                        .and_where(entities::logical_commit::Column::RepoId.eq(repo_id.to_string()))
                        .and_where(
                            entities::logical_commit::Column::Ordinal
                                .gte(usize_to_i64(commits_from, "commit ordinal")?),
                        )
                        .to_owned(),
                ),
            )
            .order_by_asc(entities::file_change::Column::CommitId)
            .order_by_asc(entities::file_change::Column::Ordinal)
            .all(conn)
            .await
            .map_err(PostgresError::internal)?
        {
            changes_by_commit
                .entry(row.commit_id.clone())
                .or_default()
                .push(file_change_from_row(row)?);
        }
    }
    let commits = commit_rows
        .into_iter()
        .map(|row| {
            let changes = changes_by_commit.remove(&row.id).unwrap_or_default();
            logical_commit_from_row(row, changes)
        })
        .collect::<Result<Vec<_>, _>>()?;

    let set_rows = entities::visibility_change_set::Entity::find()
        .filter(entities::visibility_change_set::Column::RepoId.eq(repo_id.to_string()))
        .filter(
            entities::visibility_change_set::Column::Ordinal
                .gte(usize_to_i64(change_sets_from, "change set ordinal")?),
        )
        .order_by_asc(entities::visibility_change_set::Column::Ordinal)
        .all(conn)
        .await
        .map_err(PostgresError::internal)?;
    let mut changes_by_set = BTreeMap::<String, Vec<VisibilityChange>>::new();
    if !set_rows.is_empty() {
        for row in entities::visibility_change::Entity::find()
            .filter(entities::visibility_change::Column::RepoId.eq(repo_id.to_string()))
            .filter(
                entities::visibility_change::Column::ChangeSetId.in_subquery(
                    Query::select()
                        .column(entities::visibility_change_set::Column::Id)
                        .from(entities::visibility_change_set::Entity)
                        .and_where(
                            entities::visibility_change_set::Column::RepoId.eq(repo_id.to_string()),
                        )
                        .and_where(
                            entities::visibility_change_set::Column::Ordinal
                                .gte(usize_to_i64(change_sets_from, "change set ordinal")?),
                        )
                        .to_owned(),
                ),
            )
            .order_by_asc(entities::visibility_change::Column::ChangeSetId)
            .order_by_asc(entities::visibility_change::Column::Ordinal)
            .all(conn)
            .await
            .map_err(PostgresError::internal)?
        {
            changes_by_set
                .entry(row.change_set_id.clone())
                .or_default()
                .push(visibility_change_from_row(row)?);
        }
    }
    let visibility_change_sets = set_rows
        .into_iter()
        .map(|row| {
            let changes = changes_by_set.remove(&row.id).unwrap_or_default();
            visibility_change_set_from_row(row, changes)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(AppendedRepositoryHistory {
        commits,
        visibility_change_sets,
    })
}
