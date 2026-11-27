use super::joined_rows::{joined, prefixed_columns};
use super::{
    RepositoryStore, begin_metadata_read_snapshot, entities, repository_access::load_repo_record,
};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseBackend, EntityTrait, FromQueryResult, IdenStatic,
    QueryFilter, QueryResult, QuerySelect, Statement,
};
use std::sync::OnceLock;
use {
    crate::error::PostgresError,
    scope_domain::{
        repo_config::RepoConfig,
        repository::access::{RepositoryAccess, repository_access_for_user_id},
        repository::credentials::{FirstPushToken, GitPushToken},
        repository::git::GitHead,
        repository::{RepoLifecycleState, RepoRecord, RepositoryIncarnation, repo_id},
    },
};

#[derive(Clone, Debug)]
pub struct GitPushState {
    pub repo_id: String,
    pub incarnation: RepositoryIncarnation,
    pub owner_user_id: String,
    pub lifecycle_state: RepoLifecycleState,
    pub access: RepositoryAccess,
    pub repo_config: RepoConfig,
    pub git_head: Option<GitHead>,
    pub change_version: u64,
    pub content_version: u64,
}

#[derive(Clone, Debug)]
pub struct GitPushCredentials {
    pub record: RepoRecord,
    pub first_push_token: Option<FirstPushToken>,
    pub git_push_token: Option<GitPushToken>,
}

impl RepositoryStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "git_push_state"))]
    pub async fn git_push_state(
        &self,
        owner: &str,
        name: &str,
        user_id: &str,
    ) -> Result<Option<GitPushState>, PostgresError> {
        let id = repo_id(owner, name);
        git_push_state_for_id(self.db.as_ref(), &id, user_id).await
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "has_drifted_visibility_labels"))]
    pub async fn has_drifted_visibility_labels(
        &self,
        repo_id: &str,
        config: &RepoConfig,
    ) -> Result<bool, PostgresError> {
        let tx = begin_metadata_read_snapshot(self.db.as_ref()).await?;
        let Some(policy_json) = entities::repository::Entity::find_by_id(repo_id.to_string())
            .select_only()
            .column(entities::repository::Column::Policy)
            .into_tuple::<serde_json::Value>()
            .one(&tx)
            .await?
        else {
            tx.commit().await?;
            return Ok(false);
        };
        let drifted = live_file_labels_drifted(&tx, repo_id, &policy_json, config).await?;
        tx.commit().await?;
        Ok(drifted)
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "git_push_credentials"))]
    pub async fn git_push_credentials(
        &self,
        owner: &str,
        name: &str,
    ) -> Result<Option<GitPushCredentials>, PostgresError> {
        let id = repo_id(owner, name);
        let tx = begin_metadata_read_snapshot(self.db.as_ref()).await?;
        let Some(record) = load_repo_record(&tx, &id).await? else {
            tx.commit().await?;
            return Ok(None);
        };
        let first_push_token =
            entities::repository_first_push_token::Entity::find_by_id(id.clone())
                .one(&tx)
                .await?
                .map(entities::repository_first_push_token::Model::try_into_domain)
                .transpose()?;
        let git_push_token = entities::repository_git_push_token::Entity::find_by_id(id)
            .one(&tx)
            .await?
            .map(entities::repository_git_push_token::Model::try_into_domain)
            .transpose()?;
        tx.commit().await?;
        Ok(Some(GitPushCredentials {
            record,
            first_push_token,
            git_push_token,
        }))
    }
}

pub(super) async fn live_file_labels_drifted<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
    stored_policy_json: &serde_json::Value,
    config: &RepoConfig,
) -> Result<bool, PostgresError> {
    let stored_policy: scope_domain::policy::Policy =
        serde_json::from_value(stored_policy_json.clone()).map_err(PostgresError::internal)?;
    let rows = entities::live_file::Entity::find()
        .select_only()
        .columns([
            entities::live_file::Column::Path,
            entities::live_file::Column::View,
        ])
        .filter(entities::live_file::Column::RepoId.eq(repo_id))
        .into_tuple::<(String, Option<String>)>()
        .all(conn)
        .await?;
    let mut paths = Vec::with_capacity(rows.len());
    let mut rules = Vec::new();
    for (path, view) in rows {
        let parsed_path =
            scope_domain::policy::ScopePath::parse(path).map_err(PostgresError::internal)?;
        if let Some(view) = view {
            rules.push(scope_domain::policy::LabelRule {
                path: parsed_path.clone(),
                view: scope_domain::views::ViewId::parse(&view).map_err(PostgresError::internal)?,
            });
        }
        paths.push(parsed_path);
    }
    let policy = stored_policy.join_rules(rules);
    Ok(
        !scope_domain::reviewed_updates::policy::policy_matches_config(
            &policy,
            config,
            paths.iter(),
        ),
    )
}

pub(super) async fn git_push_state_for_id<C: ConnectionTrait>(
    conn: &C,
    id: &str,
    user_id: &str,
) -> Result<Option<GitPushState>, PostgresError> {
    conn.query_one_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        git_push_state_sql(),
        [id.into(), user_id.into()],
    ))
    .await?
    .map(|row| git_push_state_from_row(&row, id, user_id))
    .transpose()
}

fn git_push_state_from_row(
    row: &QueryResult,
    id: &str,
    user_id: &str,
) -> Result<GitPushState, PostgresError> {
    let repo_row = PushRepositoryRow::from_query_result(row, "r_")?;
    let git_head = if joined(row, "h_repo_id")? {
        Some(entities::git_head::Model::from_query_result(row, "h_")?.try_into_domain()?)
    } else {
        None
    };
    let permissions = if joined(row, "m_repo_id")? {
        Some(
            entities::repository_member::Model::from_query_result(row, "m_")?
                .try_into_domain()?
                .permissions,
        )
    } else {
        None
    };
    let lifecycle_state = entities::decode_enum(repo_row.publication_state)?;
    let access = repository_access_for_user_id(
        &repo_row.owner_user_id,
        lifecycle_state,
        permissions,
        user_id,
    );
    Ok(GitPushState {
        incarnation: RepositoryIncarnation::new(id, repo_row.incarnation_id)
            .map_err(PostgresError::internal)?,
        repo_id: id.to_string(),
        owner_user_id: repo_row.owner_user_id,
        lifecycle_state,
        access,
        repo_config: serde_json::from_value(repo_row.repo_config)
            .map_err(PostgresError::internal)?,
        git_head,
        change_version: super::integer_columns::i64_to_u64(
            repo_row.change_version,
            "repository change version",
        )?,
        content_version: super::integer_columns::i64_to_u64(
            repo_row.content_version,
            "repository content version",
        )?,
    })
}

#[derive(FromQueryResult)]
struct PushRepositoryRow {
    incarnation_id: String,
    owner_user_id: String,
    publication_state: String,
    change_version: i64,
    content_version: i64,
    repo_config: serde_json::Value,
}

fn git_push_state_columns() -> String {
    use entities::repository::Column;
    [
        [
            Column::IncarnationId,
            Column::OwnerUserId,
            Column::PublicationState,
            Column::ChangeVersion,
            Column::ContentVersion,
            Column::RepoConfig,
        ]
        .iter()
        .map(|column| format!("repo.\"{0}\" AS \"r_{0}\"", column.as_str()))
        .collect::<Vec<_>>()
        .join(", "),
        prefixed_columns::<entities::git_head::Column>("head", "h_"),
        prefixed_columns::<entities::repository_member::Column>("member", "m_"),
    ]
    .join(", ")
}

const GIT_PUSH_STATE_FROM: &str = "FROM scope_repositories repo
     LEFT JOIN scope_git_heads head ON head.repo_id = repo.id
     LEFT JOIN scope_repository_members member
       ON member.repo_id = repo.id AND member.user_id = $2
     WHERE repo.id = $1";

fn git_push_state_sql() -> &'static str {
    static SQL: OnceLock<String> = OnceLock::new();
    SQL.get_or_init(|| format!("SELECT {} {GIT_PUSH_STATE_FROM}", git_push_state_columns()))
}
