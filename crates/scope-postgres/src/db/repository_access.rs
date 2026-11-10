use super::{RepositoryStore, begin_metadata_read_snapshot, entities};
use super::{
    integer_columns,
    joined_rows::{joined, prefixed_columns},
};
use crate::error::PostgresError;
use scope_domain::{
    repository::{
        RepoRecord,
        access::{RepositoryAccess, RepositoryAccessContext, repository_access_for_user_id},
        repo_id,
    },
    views::ViewId,
};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseBackend, DatabaseTransaction, EntityTrait,
    FromQueryResult, IdenStatic, QueryFilter, QueryResult, QuerySelect, Statement,
};

#[derive(FromQueryResult)]
struct AccessRow {
    id: String,
    incarnation_id: String,
    owner_handle: String,
    name: String,
    owner_user_id: String,
    description: Option<String>,
    website_url: Option<String>,
    publication_state: String,
    change_version: i64,
    content_version: i64,
}

#[derive(Clone, Debug)]
pub struct RepositoryReadPolicy {
    pub context: RepositoryAccessContext,
    pub policy: scope_domain::policy::Policy,
    pub repo_config: scope_domain::repo_config::RepoConfig,
}

impl RepositoryStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "repository_read_access"))]
    pub async fn repository_read_access(
        &self,
        owner: &str,
        name: &str,
        viewer_user_id: Option<&str>,
    ) -> Result<Option<RepositoryAccessContext>, PostgresError> {
        for _ in 0..3 {
            let Some(read) =
                load_access(self.db.as_ref(), &repo_id(owner, name), viewer_user_id).await?
            else {
                return Ok(None);
            };
            let Some(public_files_visible) = read.public_files_visible() else {
                self.ensure_live_projection_read_models(
                    &read.context.incarnation(),
                    read.context.views.anyone().expect("public view exists"),
                )
                .await?;
                continue;
            };
            return Ok(read
                .context
                .can_read(public_files_visible)
                .then_some(read.context));
        }
        Err(PostgresError::conflict(
            "repository kept changing while reading access; retry",
        ))
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "repository_read_policy"))]
    pub async fn repository_read_policy(
        &self,
        owner: &str,
        name: &str,
        viewer_user_id: Option<&str>,
    ) -> Result<Option<RepositoryReadPolicy>, PostgresError> {
        let Some((tx, context)) = self
            .begin_read_access_snapshot(&repo_id(owner, name), viewer_user_id)
            .await?
        else {
            return Ok(None);
        };
        let (policy, repo_config) = load_policy_config(&tx, &context.record.id).await?;
        tx.commit().await?;
        Ok(Some(RepositoryReadPolicy {
            context,
            policy,
            repo_config,
        }))
    }

    pub(super) async fn begin_read_access_snapshot(
        &self,
        repo_id: &str,
        viewer_user_id: Option<&str>,
    ) -> Result<Option<(DatabaseTransaction, RepositoryAccessContext)>, PostgresError> {
        let Some((tx, context, public_files_visible)) =
            self.begin_access_snapshot(repo_id, viewer_user_id).await?
        else {
            return Ok(None);
        };
        if !context.can_read(public_files_visible) {
            tx.commit().await?;
            return Ok(None);
        }
        Ok(Some((tx, context)))
    }

    pub(super) async fn begin_access_snapshot(
        &self,
        repo_id: &str,
        viewer_user_id: Option<&str>,
    ) -> Result<Option<(DatabaseTransaction, RepositoryAccessContext, bool)>, PostgresError> {
        for _ in 0..3 {
            let tx = begin_metadata_read_snapshot(self.db.as_ref()).await?;
            let Some(read) = load_access(&tx, repo_id, viewer_user_id).await? else {
                tx.commit().await?;
                return Ok(None);
            };
            if read.context.access.actor
                == scope_domain::repository::access::RepositoryActor::Public
                && read.context.record.lifecycle_state
                    == scope_domain::repository::RepoLifecycleState::Ready
                && read.context.views.anyone().is_none()
            {
                tx.commit().await?;
                return Ok(None);
            }
            let Some(public_files_visible) = read.public_files_visible() else {
                tx.commit().await?;
                self.ensure_live_projection_read_models(
                    &read.context.incarnation(),
                    read.context.views.anyone().expect("public view exists"),
                )
                .await?;
                continue;
            };
            let context = read.context;
            return Ok(Some((tx, context, public_files_visible)));
        }
        Err(PostgresError::conflict(
            "repository kept changing while reading access; retry",
        ))
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "repository_access"))]
    pub async fn repository_access(
        &self,
        owner: &str,
        name: &str,
        viewer_user_id: Option<&str>,
    ) -> Result<Option<RepositoryAccessContext>, PostgresError> {
        repository_access(self.db.as_ref(), &repo_id(owner, name), viewer_user_id).await
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "repository_record"))]
    pub async fn repository_record(
        &self,
        repo_id: &str,
    ) -> Result<Option<RepoRecord>, PostgresError> {
        load_repo_record(self.db.as_ref(), repo_id).await
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "repository_content_source"))]
    pub async fn repository_content_source(
        &self,
        incarnation: &scope_domain::repository::RepositoryIncarnation,
    ) -> Result<Option<scope_domain::repository::git::GitHead>, PostgresError> {
        Ok(self.repository_git_state(incarnation).await?.git_head)
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "repository_policy"))]
    pub async fn repository_policy(
        &self,
        context: &RepositoryAccessContext,
    ) -> Result<scope_domain::policy::Policy, PostgresError> {
        Ok(self.repository_policy_config(context).await?.policy)
    }

    pub async fn repository_policy_config(
        &self,
        context: &RepositoryAccessContext,
    ) -> Result<RepositoryReadPolicy, PostgresError> {
        let tx = begin_metadata_read_snapshot(self.db.as_ref()).await?;
        let current = repository_access(&tx, &context.record.id, None)
            .await?
            .ok_or_else(|| PostgresError::not_found("repo not found"))?;
        ensure_current_context(context, &current)?;
        let (policy, repo_config) = load_policy_config(&tx, &context.record.id).await?;
        tx.commit().await?;
        Ok(RepositoryReadPolicy {
            context: context.clone(),
            policy,
            repo_config,
        })
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "repository_main_oid"))]
    pub async fn repository_main_oid(
        &self,
        context: &RepositoryAccessContext,
    ) -> Result<Option<String>, PostgresError> {
        self.repository_main_oid_for_view(context, &context.access.view)
            .await
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "repository_main_oid_for_view"))]
    pub async fn repository_main_oid_for_view(
        &self,
        context: &RepositoryAccessContext,
        view: &ViewId,
    ) -> Result<Option<String>, PostgresError> {
        for _ in 0..2 {
            let tx = begin_metadata_read_snapshot(self.db.as_ref()).await?;
            let current = repository_access(&tx, &context.record.id, None)
                .await?
                .ok_or_else(|| PostgresError::not_found("repo not found"))?;
            ensure_current_context(context, &current)?;
            if !context.access.can_read_view(&current.views, view) {
                return Err(PostgresError::permission_denied(
                    "repository view requires access",
                ));
            }
            if view == current.views.full()
                && let Some(head) = entities::git_head::Entity::find_by_id(&context.record.id)
                    .one(&tx)
                    .await?
            {
                tx.commit().await?;
                return Ok(Some(head.head_oid));
            }
            let metadata = super::projection_read_models::live_projection_read_model(
                &tx,
                &context.record.id,
                context.record.content_version,
                view,
            )
            .await?;
            tx.commit().await?;
            if let Some(view) = metadata {
                return Ok(view.head_oid);
            }
            self.ensure_live_projection_read_models(&context.incarnation(), view)
                .await?;
        }
        Err(PostgresError::conflict(
            "repository changed while reading its head; retry",
        ))
    }
}

pub(super) async fn repository_access<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
    viewer_user_id: Option<&str>,
) -> Result<Option<RepositoryAccessContext>, PostgresError> {
    query_access(conn, AccessQuery::Membership, repo_id, viewer_user_id)
        .await?
        .first()
        .map(|row| access_context(row, viewer_user_id))
        .transpose()
}

pub(super) struct RepositoryAccessRead {
    pub context: RepositoryAccessContext,
    projections: Vec<entities::projection_read_model::Model>,
}

impl RepositoryAccessRead {
    pub fn projection(&self, view: &ViewId) -> Option<&entities::projection_read_model::Model> {
        self.projections.iter().find(|row| {
            row.audience == view.as_str()
                && row.current()
                && u64::try_from(row.repo_version).ok() == Some(self.context.record.content_version)
        })
    }

    fn public_files_visible(&self) -> Option<bool> {
        if self.context.access.actor != scope_domain::repository::access::RepositoryActor::Public
            || self.context.record.lifecycle_state
                != scope_domain::repository::RepoLifecycleState::Ready
        {
            return Some(false);
        }
        let Some(view) = self.context.views.anyone() else {
            return Some(false);
        };
        self.projection(view)
            .map(|projection| projection.visible_files)
    }
}

pub(super) async fn load_access<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
    viewer_user_id: Option<&str>,
) -> Result<Option<RepositoryAccessRead>, PostgresError> {
    let rows = query_access(
        conn,
        AccessQuery::MembershipAndProjections,
        repo_id,
        viewer_user_id,
    )
    .await?;
    let Some(row) = rows.first() else {
        return Ok(None);
    };
    let context = access_context(row, viewer_user_id)?;
    let projections = rows
        .iter()
        .filter_map(|row| match joined(row, "p_repo_id") {
            Ok(true) => Some(
                entities::projection_read_model::Model::from_query_result(row, "p_")
                    .map_err(PostgresError::from),
            ),
            Ok(false) => None,
            Err(error) => Some(Err(error)),
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some(RepositoryAccessRead {
        context,
        projections,
    }))
}

#[derive(Clone, Copy)]
enum AccessQuery {
    Membership,
    MembershipAndProjections,
}

async fn query_access<C: ConnectionTrait>(
    conn: &C,
    query: AccessQuery,
    repo_id: &str,
    viewer_user_id: Option<&str>,
) -> Result<Vec<QueryResult>, PostgresError> {
    Ok(conn
        .query_all_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            access_sql(query),
            [repo_id.into(), viewer_user_id.into()],
        ))
        .await?)
}

fn access_context(
    row: &QueryResult,
    viewer_user_id: Option<&str>,
) -> Result<RepositoryAccessContext, PostgresError> {
    let record = AccessRow::from_query_result(row, "r_")?.try_into_domain()?;
    let permissions = if joined(row, "m_repo_id")? {
        Some(
            entities::repository_member::Model::from_query_result(row, "m_")?
                .try_into_domain()?
                .permissions,
        )
    } else {
        None
    };
    let access = viewer_user_id.map_or_else(RepositoryAccess::public, |user_id| {
        repository_access_for_user_id(
            &record.owner_user_id,
            record.lifecycle_state,
            permissions,
            user_id,
        )
    });
    let config: scope_domain::repo_config::RepoConfig =
        super::decode_json(row.try_get("", "repo_config")?)?;
    Ok(RepositoryAccessContext {
        record,
        access,
        views: config.views,
    })
}

fn access_columns() -> [entities::repository::Column; 10] {
    use entities::repository::Column;
    [
        Column::Id,
        Column::IncarnationId,
        Column::OwnerHandle,
        Column::Name,
        Column::OwnerUserId,
        Column::Description,
        Column::WebsiteUrl,
        Column::PublicationState,
        Column::ChangeVersion,
        Column::ContentVersion,
    ]
}

fn access_sql(query: AccessQuery) -> &'static str {
    static MEMBERSHIP: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    static MEMBERSHIP_AND_PROJECTIONS: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let render = |projections: bool| {
        let membership = format!(
            "SELECT {}, repo.repo_config, {}",
            access_columns()
                .iter()
                .map(|column| format!("repo.\"{0}\" AS \"r_{0}\"", column.as_str()))
                .collect::<Vec<_>>()
                .join(", "),
            prefixed_columns::<entities::repository_member::Column>("member", "m_"),
        );
        let (projection_columns, projection_join) = if projections {
            (
                format!(
                    ", {}",
                    prefixed_columns::<entities::projection_read_model::Column>("projection", "p_")
                ),
                "LEFT JOIN scope_projection_read_models projection ON projection.repo_id = repo.id",
            )
        } else {
            (String::new(), "")
        };
        format!(
            "{membership}{projection_columns}
             FROM scope_repositories repo
             LEFT JOIN scope_repository_members member
               ON member.repo_id = repo.id AND member.user_id = $2 AND member.user_id <> repo.owner_user_id
             {projection_join}
             WHERE repo.id = $1"
        )
    };
    match query {
        AccessQuery::Membership => MEMBERSHIP.get_or_init(|| render(false)),
        AccessQuery::MembershipAndProjections => {
            MEMBERSHIP_AND_PROJECTIONS.get_or_init(|| render(true))
        }
    }
}

async fn load_policy_config<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
) -> Result<
    (
        scope_domain::policy::Policy,
        scope_domain::repo_config::RepoConfig,
    ),
    PostgresError,
> {
    let (policy, config) = entities::repository::Entity::find_by_id(repo_id)
        .select_only()
        .columns([
            entities::repository::Column::Policy,
            entities::repository::Column::RepoConfig,
        ])
        .into_tuple::<(serde_json::Value, serde_json::Value)>()
        .one(conn)
        .await?
        .ok_or_else(|| PostgresError::not_found("repo not found"))?;
    let policy = serde_json::from_value::<scope_domain::policy::Policy>(policy)
        .map_err(PostgresError::internal)?
        .join_rules(super::history_rows::load_live_file_rules(conn, repo_id).await?);
    Ok((
        policy,
        serde_json::from_value(config).map_err(PostgresError::internal)?,
    ))
}

pub(super) async fn load_repo_record<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
) -> Result<Option<RepoRecord>, PostgresError> {
    use entities::repository::{Column, Entity};
    let Some(row) = Entity::find()
        .select_only()
        .columns(access_columns())
        .filter(Column::Id.eq(repo_id))
        .into_model::<AccessRow>()
        .one(conn)
        .await?
    else {
        return Ok(None);
    };
    row.try_into_domain().map(Some)
}

impl AccessRow {
    fn try_into_domain(self) -> Result<RepoRecord, PostgresError> {
        Ok(RepoRecord {
            id: self.id,
            incarnation_id: self.incarnation_id,
            owner_handle: self.owner_handle,
            name: self.name,
            owner_user_id: self.owner_user_id,
            description: self.description,
            website_url: self.website_url,
            lifecycle_state: entities::decode_enum(self.publication_state)?,
            change_version: integer_columns::i64_to_u64(
                self.change_version,
                "repository change version",
            )?,
            content_version: integer_columns::i64_to_u64(
                self.content_version,
                "repository content version",
            )?,
        })
    }
}

fn ensure_current_context(
    expected: &RepositoryAccessContext,
    current: &RepositoryAccessContext,
) -> Result<(), PostgresError> {
    if current.incarnation() != expected.incarnation()
        || current.record.change_version != expected.record.change_version
    {
        return Err(PostgresError::conflict(
            "repository changed while reading metadata; retry",
        ));
    }
    Ok(())
}
