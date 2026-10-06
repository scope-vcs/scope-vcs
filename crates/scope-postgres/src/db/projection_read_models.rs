use super::{
    entities,
    history_rows::{history_position_matches, load_repository_history_after},
    integer_columns::{u64_to_i64, usize_to_i64},
};
use sea_orm::{
    ColumnTrait, Condition, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter, QueryOrder,
    sea_query::OnConflict,
};
use {
    crate::error::PostgresError,
    scope_domain::{
        content::SourceBlob,
        history::{HistoryCursor, history_entries_after},
        policy::{ScopePath, Visibility},
        projection::{
            ProjectionCursor, ProjectionViewKey, SourceGraph, project_graph_after,
            projection_delta_appends,
        },
        projection_views::{ProjectionViewFile, ProjectionViewFileContent},
        repo_control::{
            LEGACY_REPO_RULES_PATH, REPO_CONTROL_PREFIX, REPO_CONTROL_ROOT, is_repo_control_path,
        },
        visibility_changes::VisibilityChangeSet,
    },
    std::collections::BTreeMap,
};

const PROJECTION_FILE_INSERT_BATCH_SIZE: usize = 1_000;
const VIEWS: [ProjectionViewKey; 2] = [ProjectionViewKey::Private, ProjectionViewKey::Public];

pub(super) enum ProjectionFileLookup {
    Found(ProjectionViewFileContent),
    Missing,
    NotReady,
}

type LabelledFiles = BTreeMap<ScopePath, (SourceBlob, Visibility)>;

enum FoldOutcome {
    Folded,
    Diverged(String),
}

pub async fn fold_live_projection_read_models<C>(
    conn: &C,
    repo_id: &str,
    content_version: u64,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    let rows = entities::projection_read_model::Entity::find()
        .filter(entities::projection_read_model::Column::RepoId.eq(repo_id.to_string()))
        .all(conn)
        .await
        .map_err(PostgresError::internal)?;
    if let Some(rows) = resumable(rows)? {
        match fold(conn, repo_id, content_version, Some(rows)).await? {
            FoldOutcome::Folded => return Ok(()),
            FoldOutcome::Diverged(reason) => {
                tracing::warn!(
                    repo_id,
                    reason,
                    "rebuilding projection read models from scratch"
                );
            }
        }
    }
    reset_live_projection_read_models(conn, repo_id).await?;
    match fold(conn, repo_id, content_version, None).await? {
        FoldOutcome::Folded => Ok(()),
        FoldOutcome::Diverged(reason) => Err(PostgresError::internal_message(reason)),
    }
}

fn resumable(
    rows: Vec<entities::projection_read_model::Model>,
) -> Result<Option<Vec<entities::projection_read_model::Model>>, PostgresError> {
    if rows.len() != VIEWS.len()
        || !VIEWS
            .iter()
            .all(|view| rows.iter().any(|row| row.audience == view.as_str()))
        || !rows.iter().all(|row| row.current())
    {
        return Ok(None);
    }
    let position = rows[0].position()?;
    for row in &rows[1..] {
        if row.position()? != position {
            return Ok(None);
        }
    }
    Ok(Some(rows))
}

pub(super) async fn reset_live_projection_read_models<C>(
    conn: &C,
    repo_id: &str,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    entities::projection_read_model::Entity::delete_many()
        .filter(entities::projection_read_model::Column::RepoId.eq(repo_id.to_string()))
        .exec(conn)
        .await
        .map_err(PostgresError::internal)?;
    super::history_reads::delete_history_payloads(conn, repo_id).await?;
    entities::projection_file::Entity::delete_many()
        .filter(entities::projection_file::Column::RepoId.eq(repo_id.to_string()))
        .exec(conn)
        .await
        .map_err(PostgresError::internal)?;
    Ok(())
}

async fn fold<C>(
    conn: &C,
    repo_id: &str,
    content_version: u64,
    resumed: Option<Vec<entities::projection_read_model::Model>>,
) -> Result<FoldOutcome, PostgresError>
where
    C: ConnectionTrait,
{
    let position = match &resumed {
        Some(rows) => rows[0].position()?,
        None => Default::default(),
    };
    let appended =
        load_repository_history_after(conn, repo_id, position.commits, position.change_sets)
            .await?;
    if resumed.is_some()
        && (!history_position_matches(conn, repo_id, &position).await?
            || !projection_delta_appends(
                position.last_commit_id.as_deref(),
                &appended.commits,
                &appended.visibility_change_sets,
            ))
    {
        return Ok(FoldOutcome::Diverged(
            "repository history no longer continues the folded read models".to_string(),
        ));
    }
    let graph = SourceGraph {
        repo_id: repo_id.to_string(),
        commits: appended.commits,
    };
    let sets = appended.visibility_change_sets;

    let before = match &resumed {
        Some(_) => load_projection_files(conn, repo_id).await?,
        None => LabelledFiles::new(),
    };
    let after = label_files(before.clone(), &graph, &sets);

    for view in VIEWS {
        let row = resumed
            .as_ref()
            .and_then(|rows| rows.iter().find(|row| row.audience == view.as_str()));
        let cursor = row
            .map(entities::projection_read_model::Model::projection_cursor)
            .transpose()?
            .unwrap_or_default();
        let view_before = view_files(&before, view);
        let projection = project_graph_after(&cursor, &graph, &sets, view);
        let mut head = scope_git::ProjectionHead::resume(
            row.and_then(|row| row.head_oid.clone()),
            view_before.iter(),
        )
        .map_err(PostgresError::internal)?;
        head.apply(&projection.commits)
            .map_err(PostgresError::internal)?;
        let mut projected = view_before.clone();
        projection.apply_to(&mut projected);
        if projected != view_files(&after, view) {
            return Ok(FoldOutcome::Diverged(format!(
                "{} projection does not show the files its labels select",
                view.as_str()
            )));
        }
        let next_cursor = ProjectionCursor {
            commit_count: cursor.commit_count + projection.commits.len(),
            last_projected_id: projection
                .commits
                .last()
                .map(|commit| commit.projected_id.clone())
                .or(cursor.last_projected_id),
        };
        let mut history = row
            .map(entities::projection_read_model::Model::history_cursor)
            .unwrap_or_else(|| HistoryCursor::start(repo_id, view));
        let entries = history_entries_after(&mut history, view_before, projection, &graph, &sets);
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
                ])
                .to_owned(),
            )
            .exec(conn)
            .await
            .map_err(PostgresError::internal)?;
        super::history_reads::append_history_entries(conn, repo_id, view, first_position, &entries)
            .await?;
    }

    save_projection_files(conn, repo_id, &before, &after).await?;
    Ok(FoldOutcome::Folded)
}

fn label_files(
    mut files: LabelledFiles,
    graph: &SourceGraph,
    sets: &[VisibilityChangeSet],
) -> LabelledFiles {
    let appended = |anchor: Option<&str>| {
        anchor.is_some_and(|anchor| graph.commits.iter().any(|commit| commit.id == anchor))
    };
    relabel(
        &mut files,
        sets.iter()
            .filter(|set| !appended(set.anchor_commit_id.as_deref())),
    );
    for commit in &graph.commits {
        for change in &commit.changes {
            match &change.new_content {
                Some(blob) => files.insert(change.path.clone(), (blob.clone(), change.visibility)),
                None => files.remove(&change.path),
            };
        }
        relabel(
            &mut files,
            sets.iter()
                .filter(|set| set.anchor_commit_id.as_deref() == Some(&commit.id)),
        );
    }
    files
}

fn relabel<'a>(files: &mut LabelledFiles, sets: impl Iterator<Item = &'a VisibilityChangeSet>) {
    for change in sets.flat_map(|set| &set.changes) {
        if let Some((_, label)) = files.get_mut(&change.path) {
            *label = change.new_visibility;
        }
    }
}

fn view_files(files: &LabelledFiles, view: ProjectionViewKey) -> BTreeMap<ScopePath, SourceBlob> {
    files
        .iter()
        .filter(|(path, (_, label))| view.shows(path, *label))
        .map(|(path, (blob, _))| (path.clone(), blob.clone()))
        .collect()
}

fn view_condition(view: ProjectionViewKey) -> Result<Condition, PostgresError> {
    let labels = view
        .labels()
        .iter()
        .map(|label| entities::encode_enum(*label))
        .collect::<Result<Vec<_>, _>>()?;
    let mut condition =
        Condition::all().add(entities::projection_file::Column::Visibility.is_in(labels));
    if view != ProjectionViewKey::Private {
        condition = condition.add(
            Condition::any()
                .add(entities::projection_file::Column::Path.eq(LEGACY_REPO_RULES_PATH))
                .add(
                    Condition::all()
                        .add(entities::projection_file::Column::Path.ne(REPO_CONTROL_ROOT))
                        .add(
                            entities::projection_file::Column::Path
                                .not_like(format!("{REPO_CONTROL_PREFIX}%")),
                        ),
                ),
        );
    }
    Ok(condition)
}

async fn load_projection_files<C>(conn: &C, repo_id: &str) -> Result<LabelledFiles, PostgresError>
where
    C: ConnectionTrait,
{
    entities::projection_file::Entity::find()
        .filter(entities::projection_file::Column::RepoId.eq(repo_id.to_string()))
        .all(conn)
        .await
        .map_err(PostgresError::internal)?
        .into_iter()
        .map(|row| {
            let content = row.try_into_content()?;
            Ok((content.file.path, (content.blob, content.file.visibility)))
        })
        .collect()
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
                        visibility: *label,
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

pub(super) async fn live_projection_read_model<C>(
    conn: &C,
    repo_id: &str,
    repo_version: u64,
    view: ProjectionViewKey,
) -> Result<Option<entities::projection_read_model::Model>, PostgresError>
where
    C: ConnectionTrait,
{
    let repo_version = u64_to_i64(repo_version, "repository version")?;
    Ok(entities::projection_read_model::Entity::find_by_id((
        repo_id.to_string(),
        view.as_str().to_string(),
    ))
    .one(conn)
    .await
    .map_err(PostgresError::internal)?
    .filter(|row| row.current() && row.repo_version == repo_version))
}

pub(super) async fn load_live_projection_file_for_audience<C>(
    conn: &C,
    repo_id: &str,
    repo_version: u64,
    view: ProjectionViewKey,
    path: &ScopePath,
) -> Result<ProjectionFileLookup, PostgresError>
where
    C: ConnectionTrait,
{
    if live_projection_read_model(conn, repo_id, repo_version, view)
        .await?
        .is_none()
    {
        return Ok(ProjectionFileLookup::NotReady);
    }
    let row = entities::projection_file::Entity::find_by_id((
        repo_id.to_string(),
        entities::projection_file::projection_file_path_key(path),
    ))
    .filter(entities::projection_file::Column::Path.eq(path.as_str().to_string()))
    .one(conn)
    .await
    .map_err(PostgresError::internal)?;
    match row {
        Some(row) => {
            let content = row.try_into_content()?;
            if view.shows(&content.file.path, content.file.visibility) {
                Ok(ProjectionFileLookup::Found(content))
            } else {
                Ok(ProjectionFileLookup::Missing)
            }
        }
        None => Ok(ProjectionFileLookup::Missing),
    }
}

pub(super) async fn load_live_projection_files_for_audience<C>(
    conn: &C,
    repo_id: &str,
    repo_version: u64,
    view: ProjectionViewKey,
) -> Result<Option<Vec<ProjectionViewFile>>, PostgresError>
where
    C: ConnectionTrait,
{
    let Some(model) = live_projection_read_model(conn, repo_id, repo_version, view).await? else {
        return Ok(None);
    };
    let rows = entities::projection_file::Entity::find()
        .filter(entities::projection_file::Column::RepoId.eq(repo_id.to_string()))
        .filter(view_condition(view)?)
        .order_by_asc(entities::projection_file::Column::Path)
        .all(conn)
        .await
        .map_err(PostgresError::internal)?;
    let expected_file_count = usize::try_from(model.file_count)
        .map_err(|_| PostgresError::internal_message("projection file count cannot be negative"))?;
    if rows.len() != expected_file_count {
        return Ok(None);
    }

    let mut files = Vec::with_capacity(rows.len());
    for row in rows {
        let row_path = row.path.clone();
        match row.try_into_view() {
            Ok(file) => files.push(file),
            Err(error) => {
                tracing::warn!(
                    repo_id,
                    path = %row_path,
                    error = %error.message,
                    "ignoring invalid projection read-model row"
                );
                return Ok(None);
            }
        }
    }
    Ok(Some(files))
}
