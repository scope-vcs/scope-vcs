use super::PERSISTENCE_BATCH_SIZE;
use crate::db::entities::live_file;
use crate::error::PostgresError;
use scope_domain::{
    content::SourceBlob,
    policy::{LabelRule, Policy, ScopePath},
    views::ViewId,
};
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter, QueryOrder,
    QuerySelect,
};
use std::collections::BTreeMap;

#[derive(Debug, Default)]
pub struct LiveFiles {
    pub files: BTreeMap<ScopePath, SourceBlob>,
    pub rules: Vec<LabelRule>,
}

impl LiveFiles {
    fn insert(&mut self, row: live_file::Model) -> Result<(), PostgresError> {
        let path = ScopePath::parse(row.path).map_err(PostgresError::internal)?;
        if let Some(view) = row.view {
            self.rules.push(LabelRule {
                path: path.clone(),
                view: ViewId::parse(&view).map_err(PostgresError::internal)?,
            });
        }
        self.files.insert(
            path,
            serde_json::from_value(row.content).map_err(PostgresError::internal)?,
        );
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct LiveFileRow<'a> {
    pub content: &'a SourceBlob,
    pub view: Option<&'a ViewId>,
}

pub(crate) fn file_rules(rules: &[LabelRule]) -> BTreeMap<&ScopePath, &ViewId> {
    rules.iter().map(|rule| (&rule.path, &rule.view)).collect()
}

pub(crate) fn policy_without_file_rules(
    policy: &Policy,
    live_files: &BTreeMap<ScopePath, SourceBlob>,
) -> Policy {
    policy
        .clone()
        .split_rules(|path| live_files.contains_key(path))
        .0
}

pub(super) async fn load_repositories_live_files<C>(
    conn: &C,
    repo_ids: &[String],
) -> Result<BTreeMap<String, LiveFiles>, PostgresError>
where
    C: ConnectionTrait,
{
    let mut live_files = BTreeMap::<String, LiveFiles>::new();
    if repo_ids.is_empty() {
        return Ok(live_files);
    }
    for row in live_file::Entity::find()
        .filter(live_file::Column::RepoId.is_in(repo_ids.to_vec()))
        .order_by_asc(live_file::Column::RepoId)
        .order_by_asc(live_file::Column::Path)
        .all(conn)
        .await?
    {
        live_files
            .entry(row.repo_id.clone())
            .or_default()
            .insert(row)?;
    }
    Ok(live_files)
}

pub(crate) async fn load_live_files_at<C>(
    conn: &C,
    repo_id: &str,
    paths: impl IntoIterator<Item = &ScopePath>,
) -> Result<LiveFiles, PostgresError>
where
    C: ConnectionTrait,
{
    let mut live_files = LiveFiles::default();
    for row in live_file::Entity::find()
        .filter(live_file::Column::RepoId.eq(repo_id))
        .filter(live_file::Column::Path.is_in(paths.into_iter().map(ScopePath::as_str)))
        .all(conn)
        .await?
    {
        live_files.insert(row)?;
    }
    Ok(live_files)
}

pub(crate) async fn load_live_file_rules<C>(
    conn: &C,
    repo_id: &str,
) -> Result<Vec<LabelRule>, PostgresError>
where
    C: ConnectionTrait,
{
    live_file::Entity::find()
        .select_only()
        .columns([live_file::Column::Path, live_file::Column::View])
        .filter(live_file::Column::RepoId.eq(repo_id))
        .filter(live_file::Column::View.is_not_null())
        .into_tuple::<(String, String)>()
        .all(conn)
        .await?
        .into_iter()
        .map(|(path, view)| {
            Ok(LabelRule {
                path: ScopePath::parse(path).map_err(PostgresError::internal)?,
                view: ViewId::parse(&view).map_err(PostgresError::internal)?,
            })
        })
        .collect()
}

pub async fn insert_repository_live_files<C>(
    conn: &C,
    repo_id: &str,
    live_files: &BTreeMap<ScopePath, SourceBlob>,
    policy: &Policy,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    let rules = file_rules(policy.rules());
    save_live_files(
        conn,
        repo_id,
        live_files.iter().map(|(path, content)| {
            (
                path,
                Some(LiveFileRow {
                    content,
                    view: rules.get(path).copied(),
                }),
            )
        }),
    )
    .await
}

pub(super) async fn replace_live_files<C>(
    conn: &C,
    repo_id: &str,
    live_files: &BTreeMap<ScopePath, SourceBlob>,
    policy: &Policy,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    live_file::Entity::delete_many()
        .filter(live_file::Column::RepoId.eq(repo_id.to_string()))
        .exec(conn)
        .await?;
    insert_repository_live_files(conn, repo_id, live_files, policy).await
}

pub(crate) async fn save_live_files<'a, C, I>(
    conn: &C,
    repo_id: &str,
    files: I,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
    I: IntoIterator<Item = (&'a ScopePath, Option<LiveFileRow<'a>>)>,
{
    let files = files.into_iter().collect::<Vec<_>>();
    let rows = files
        .iter()
        .filter_map(|(path, row)| row.map(|row| (*path, row)))
        .map(|(path, row)| {
            Ok(live_file::Model {
                repo_id: repo_id.to_string(),
                path: path.as_str().to_string(),
                content: serde_json::to_value(row.content).map_err(PostgresError::internal)?,
                view: row.view.map(|view| view.as_str().to_string()),
            }
            .into_active_model())
        })
        .collect::<Result<Vec<_>, PostgresError>>()?;
    for batch in files.chunks(PERSISTENCE_BATCH_SIZE) {
        live_file::Entity::delete_many()
            .filter(live_file::Column::RepoId.eq(repo_id.to_string()))
            .filter(
                live_file::Column::Path
                    .is_in(batch.iter().map(|(path, _)| path.as_str().to_string())),
            )
            .exec(conn)
            .await?;
    }
    for batch in rows.chunks(PERSISTENCE_BATCH_SIZE) {
        live_file::Entity::insert_many(batch.iter().cloned())
            .exec(conn)
            .await?;
    }
    Ok(())
}
