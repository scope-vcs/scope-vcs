use super::{
    super::{
        entities::{self, projection_read_model::FoldPosition},
        history_rows::{history_position_matches, load_repository_history_after},
        integer_columns::{u64_to_i64, usize_to_i64},
    },
    load_projection_files, repository_views,
};
use crate::error::PostgresError;
use scope_domain::{
    history::{HistoryCursor, history_entries_after},
    projection::{
        LabelledFiles, LabelledTree, ProjectionCursor, SourceGraph, initial_views,
        project_graph_after, projection_delta_appends,
    },
    projection_views::{ProjectionViewFile, ProjectionViewFileContent},
    repo_control::is_repo_control_path,
    views::{ViewId, Views},
    visibility_changes::VisibilityChangeSet,
};
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter, sea_query::OnConflict,
};
use std::collections::BTreeSet;

const PROJECTION_FILE_INSERT_BATCH_SIZE: usize = 1_000;

enum FoldOutcome {
    Folded,
    Diverged(String),
}

struct FoldInput {
    position: FoldPosition,
    graph: SourceGraph,
    sets: Vec<VisibilityChangeSet>,
    before: LabelledTree,
    after: LabelledTree,
}

impl FoldInput {
    async fn load<C: ConnectionTrait>(
        conn: &C,
        repo_id: &str,
        position: FoldPosition,
        before: LabelledFiles,
    ) -> Result<Self, PostgresError> {
        let appended =
            load_repository_history_after(conn, repo_id, position.commits, position.change_sets)
                .await?;
        let graph = SourceGraph {
            repo_id: repo_id.to_string(),
            commits: appended.commits,
        };
        let sets = appended.visibility_change_sets;
        let before = LabelledTree::new(before);
        let mut after = before.clone();
        after.fold(&graph, &sets);
        Ok(Self {
            position,
            graph,
            sets,
            before,
            after,
        })
    }

    async fn continues_history<C: ConnectionTrait>(
        &self,
        conn: &C,
        repo_id: &str,
    ) -> Result<bool, PostgresError> {
        Ok(
            history_position_matches(conn, repo_id, &self.position).await?
                && projection_delta_appends(
                    self.position.last_commit_id.as_deref(),
                    &self.graph.commits,
                    &self.sets,
                ),
        )
    }
}

pub async fn fold_live_projection_read_models<C>(
    conn: &C,
    repo_id: &str,
    content_version: u64,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    let views = repository_views(conn, repo_id).await?;
    let rows = rows_of_defined_views(conn, repo_id, &views).await?;
    let eager = eager_views(conn, repo_id, &views).await?;
    if let Some(rows) = resumable(rows)? {
        match fold_resumed(conn, repo_id, content_version, &views, &rows).await? {
            FoldOutcome::Folded => {
                let missing = eager
                    .into_iter()
                    .filter(|view| !rows.iter().any(|row| row.audience == view.as_str()))
                    .collect::<Vec<_>>();
                return build_views_from_start(conn, repo_id, content_version, &views, &missing)
                    .await;
            }
            FoldOutcome::Diverged(reason) => {
                tracing::warn!(
                    repo_id,
                    reason,
                    "rebuilding projection read models from scratch"
                );
            }
        }
    }
    super::reset_live_projection_read_models(conn, repo_id).await?;
    let input =
        FoldInput::load(conn, repo_id, FoldPosition::default(), LabelledFiles::new()).await?;
    for view in &eager {
        fold_from_start(conn, repo_id, content_version, &views, &input, view).await?;
    }
    save_projection_files(conn, repo_id, input.before.files(), input.after.files()).await
}

pub(in crate::db) async fn build_projection_read_model<C>(
    conn: &C,
    repo_id: &str,
    content_version: u64,
    view: &ViewId,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    let views = repository_views(conn, repo_id).await?;
    entities::projection_read_model::Entity::delete_by_id((
        repo_id.to_string(),
        view.as_str().to_string(),
    ))
    .exec(conn)
    .await
    .map_err(PostgresError::internal)?;
    build_views_from_start(
        conn,
        repo_id,
        content_version,
        &views,
        std::slice::from_ref(view),
    )
    .await
}

pub(in crate::db) async fn eager_views<C>(
    conn: &C,
    repo_id: &str,
    views: &Views,
) -> Result<Vec<ViewId>, PostgresError>
where
    C: ConnectionTrait,
{
    let assigned = entities::repository_member::Entity::find()
        .filter(entities::repository_member::Column::RepoId.eq(repo_id.to_string()))
        .all(conn)
        .await
        .map_err(PostgresError::internal)?
        .into_iter()
        .map(|row| row.try_into_domain().map(|member| member.permissions.view))
        .collect::<Result<BTreeSet<_>, _>>()?;
    Ok(views
        .iter()
        .map(|definition| &definition.id)
        .filter(|id| *id == views.full() || views.anyone() == Some(*id) || assigned.contains(*id))
        .cloned()
        .collect())
}

async fn rows_of_defined_views<C>(
    conn: &C,
    repo_id: &str,
    views: &Views,
) -> Result<Vec<entities::projection_read_model::Model>, PostgresError>
where
    C: ConnectionTrait,
{
    let (defined, removed): (Vec<_>, Vec<_>) = entities::projection_read_model::Entity::find()
        .filter(entities::projection_read_model::Column::RepoId.eq(repo_id.to_string()))
        .all(conn)
        .await
        .map_err(PostgresError::internal)?
        .into_iter()
        .partition(|row| views.iter().any(|view| view.id.as_str() == row.audience));
    if !removed.is_empty() {
        entities::projection_read_model::Entity::delete_many()
            .filter(entities::projection_read_model::Column::RepoId.eq(repo_id.to_string()))
            .filter(
                entities::projection_read_model::Column::Audience
                    .is_in(removed.into_iter().map(|row| row.audience)),
            )
            .exec(conn)
            .await
            .map_err(PostgresError::internal)?;
    }
    Ok(defined)
}

fn resumable(
    rows: Vec<entities::projection_read_model::Model>,
) -> Result<Option<Vec<entities::projection_read_model::Model>>, PostgresError> {
    let Some(first) = rows.first() else {
        return Ok(None);
    };
    if !rows
        .iter()
        .all(entities::projection_read_model::Model::current)
    {
        return Ok(None);
    }
    let position = first.position()?;
    for row in &rows[1..] {
        if row.position()? != position {
            return Ok(None);
        }
    }
    Ok(Some(rows))
}

async fn fold_resumed<C>(
    conn: &C,
    repo_id: &str,
    content_version: u64,
    views: &Views,
    rows: &[entities::projection_read_model::Model],
) -> Result<FoldOutcome, PostgresError>
where
    C: ConnectionTrait,
{
    let before = load_projection_files(conn, repo_id).await?;
    let input = FoldInput::load(conn, repo_id, rows[0].position()?, before).await?;
    if !input.continues_history(conn, repo_id).await? {
        return Ok(FoldOutcome::Diverged(
            "repository history no longer continues the folded read models".to_string(),
        ));
    }
    for row in rows {
        let view = ViewId::parse(&row.audience).map_err(PostgresError::internal)?;
        if let FoldOutcome::Diverged(reason) = fold_view(
            conn,
            repo_id,
            content_version,
            views,
            &input,
            Some(row),
            &view,
        )
        .await?
        {
            return Ok(FoldOutcome::Diverged(reason));
        }
    }
    save_projection_files(conn, repo_id, input.before.files(), input.after.files()).await?;
    Ok(FoldOutcome::Folded)
}

async fn build_views_from_start<C>(
    conn: &C,
    repo_id: &str,
    content_version: u64,
    views: &Views,
    targets: &[ViewId],
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    if targets.is_empty() {
        return Ok(());
    }
    let input =
        FoldInput::load(conn, repo_id, FoldPosition::default(), LabelledFiles::new()).await?;
    for view in targets {
        fold_from_start(conn, repo_id, content_version, views, &input, view).await?;
    }
    Ok(())
}

async fn fold_from_start<C>(
    conn: &C,
    repo_id: &str,
    content_version: u64,
    views: &Views,
    input: &FoldInput,
    view: &ViewId,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    match fold_view(conn, repo_id, content_version, views, input, None, view).await? {
        FoldOutcome::Folded => Ok(()),
        FoldOutcome::Diverged(reason) => Err(PostgresError::internal_message(reason)),
    }
}

async fn fold_view<C>(
    conn: &C,
    repo_id: &str,
    content_version: u64,
    views: &Views,
    input: &FoldInput,
    row: Option<&entities::projection_read_model::Model>,
    view: &ViewId,
) -> Result<FoldOutcome, PostgresError>
where
    C: ConnectionTrait,
{
    let FoldInput {
        position,
        graph,
        sets,
        before,
        after,
    } = input;
    let cursor = match row {
        Some(row) => row.projection_cursor()?,
        None => ProjectionCursor::start(initial_views(sets, views)),
    };
    let view_before = before.view_files(&cursor.views, view);
    let projection = project_graph_after(&cursor, before, graph, sets, view);
    let mut head = scope_git::ProjectionHead::resume(
        row.and_then(|row| row.head_oid.clone()),
        view_before.iter(),
    )
    .map_err(PostgresError::internal)?;
    head.apply(&projection.commits)
        .map_err(PostgresError::internal)?;
    let mut projected = view_before.clone();
    projection.apply_to(&mut projected);
    if projected != after.view_files(views, view) {
        return Ok(FoldOutcome::Diverged(format!(
            "{} projection does not show the files its labels select",
            view.as_str()
        )));
    }
    let next_cursor = cursor.advanced(&projection, graph, sets);
    let mut history = row
        .map(entities::projection_read_model::Model::history_cursor)
        .unwrap_or_else(|| HistoryCursor::start(repo_id, views, view));
    let entries = history_entries_after(
        &mut history,
        view_before,
        projection,
        graph,
        sets,
        views,
        view,
    );
    let first_position = row
        .map(entities::projection_read_model::Model::history_entries)
        .transpose()?
        .unwrap_or(0);
    let model = entities::projection_read_model::Model {
        repo_id: repo_id.to_string(),
        audience: view.as_str().to_string(),
        repo_version: u64_to_i64(content_version, "projection repository version")?,
        identity_version: scope_git::PROJECTION_IDENTITY_VERSION,
        history_version: scope_domain::history::HISTORY_GENERATION_VERSION.to_string(),
        folded_commits: usize_to_i64(
            position.commits + graph.commits.len(),
            "folded commit count",
        )?,
        folded_change_sets: usize_to_i64(
            position.change_sets + sets.len(),
            "folded change set count",
        )?,
        last_commit_id: graph
            .commits
            .last()
            .map(|commit| commit.id.clone())
            .or_else(|| position.last_commit_id.clone()),
        last_change_set_id: sets
            .last()
            .map(|set| set.id.clone())
            .or_else(|| position.last_change_set_id.clone()),
        projected_commits: usize_to_i64(next_cursor.commit_count, "projected commit count")?,
        last_projected_id: next_cursor.last_projected_id,
        head_oid: head.oid,
        file_count: usize_to_i64(projected.len(), "projection file count")?,
        visible_files: projected.keys().any(|path| !is_repo_control_path(path)),
        history_entries: usize_to_i64(first_position + entries.len(), "history entry count")?,
        last_history_entry_id: history.last_entry_id,
        history_generation: history.generation,
        views: super::super::encode_json(&next_cursor.views)?,
    };
    entities::projection_read_model::Entity::insert(model.into_active_model())
        .on_conflict(
            OnConflict::columns([
                entities::projection_read_model::Column::RepoId,
                entities::projection_read_model::Column::Audience,
            ])
            .update_columns([
                entities::projection_read_model::Column::RepoVersion,
                entities::projection_read_model::Column::IdentityVersion,
                entities::projection_read_model::Column::HistoryVersion,
                entities::projection_read_model::Column::FoldedCommits,
                entities::projection_read_model::Column::FoldedChangeSets,
                entities::projection_read_model::Column::LastCommitId,
                entities::projection_read_model::Column::LastChangeSetId,
                entities::projection_read_model::Column::ProjectedCommits,
                entities::projection_read_model::Column::LastProjectedId,
                entities::projection_read_model::Column::HeadOid,
                entities::projection_read_model::Column::FileCount,
                entities::projection_read_model::Column::VisibleFiles,
                entities::projection_read_model::Column::HistoryEntries,
                entities::projection_read_model::Column::LastHistoryEntryId,
                entities::projection_read_model::Column::HistoryGeneration,
                entities::projection_read_model::Column::Views,
            ])
            .to_owned(),
        )
        .exec(conn)
        .await
        .map_err(PostgresError::internal)?;
    super::super::history_reads::append_history_entries(
        conn,
        repo_id,
        view,
        first_position,
        &entries,
    )
    .await?;
    Ok(FoldOutcome::Folded)
}

async fn save_projection_files<C>(
    conn: &C,
    repo_id: &str,
    before: &LabelledFiles,
    after: &LabelledFiles,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    let removed = before
        .keys()
        .filter(|path| !after.contains_key(*path))
        .map(entities::projection_file::projection_file_path_key)
        .collect::<Vec<_>>();
    if !removed.is_empty() {
        entities::projection_file::Entity::delete_many()
            .filter(entities::projection_file::Column::RepoId.eq(repo_id.to_string()))
            .filter(entities::projection_file::Column::PathKey.is_in(removed))
            .exec(conn)
            .await
            .map_err(PostgresError::internal)?;
    }
    let rows = after
        .iter()
        .filter(|(path, entry)| before.get(*path) != Some(entry))
        .map(|(path, (blob, label))| {
            entities::projection_file::Model::live(
                repo_id,
                ProjectionViewFileContent {
                    file: ProjectionViewFile {
                        path: path.clone(),
                        oid: blob.git_oid.clone(),
                        tracked: true,
                        label: label.clone(),
                    },
                    blob: blob.clone(),
                },
            )
            .map(IntoActiveModel::into_active_model)
        })
        .collect::<Result<Vec<_>, PostgresError>>()?;
    for batch in rows.chunks(PROJECTION_FILE_INSERT_BATCH_SIZE) {
        entities::projection_file::Entity::insert_many(batch.iter().cloned())
            .on_conflict(
                OnConflict::columns([
                    entities::projection_file::Column::RepoId,
                    entities::projection_file::Column::PathKey,
                ])
                .update_columns([
                    entities::projection_file::Column::Path,
                    entities::projection_file::Column::Oid,
                    entities::projection_file::Column::Visibility,
                    entities::projection_file::Column::Sha256,
                    entities::projection_file::Column::ObjectKey,
                    entities::projection_file::Column::SizeBytes,
                    entities::projection_file::Column::GitFileMode,
                ])
                .to_owned(),
            )
            .exec(conn)
            .await
            .map_err(PostgresError::internal)?;
    }
    Ok(())
}
