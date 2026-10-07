use super::integer_columns;
use super::{RepositoryStore, begin_metadata_read_snapshot, entities};
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
    ColumnTrait, ConnectionTrait, DatabaseTransaction, EntityTrait, FromQueryResult, QueryFilter,
    QuerySelect,
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
}

impl RepositoryStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "repository_read_access"))]
    pub async fn repository_read_access(
        &self,
        owner: &str,
        name: &str,
        viewer_user_id: Option<&str>,
    ) -> Result<Option<RepositoryAccessContext>, PostgresError> {
        let Some((tx, context)) = self
            .begin_read_access_snapshot(&repo_id(owner, name), viewer_user_id)
            .await?
        else {
            return Ok(None);
        };
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(Some(context))
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
        let policy = load_policy(&tx, &context.record.id).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(Some(RepositoryReadPolicy { context, policy }))
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
            tx.commit().await.map_err(PostgresError::internal)?;
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
            let Some(context) = repository_access(&tx, repo_id, viewer_user_id).await? else {
                tx.commit().await.map_err(PostgresError::internal)?;
                return Ok(None);
            };
            let public_files_visible = if context.access.actor
                == scope_domain::repository::access::RepositoryActor::Public
                && context.record.lifecycle_state
                    == scope_domain::repository::RepoLifecycleState::Ready
            {
                let Some(public_view) = context.views.anyone() else {
                    tx.commit().await.map_err(PostgresError::internal)?;
                    return Ok(None);
                };
                let Some(view) = super::projection_read_models::live_projection_read_model(
                    &tx,
                    &context.record.id,
                    context.record.content_version,
                    public_view,
                )
                .await?
                else {
                    tx.commit().await.map_err(PostgresError::internal)?;
                    self.ensure_live_projection_read_models(&context.incarnation(), public_view)
                        .await?;
                    continue;
                };
                view.visible_files
            } else {
                false
            };
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
        let tx = begin_metadata_read_snapshot(self.db.as_ref()).await?;
        let context = repository_access(&tx, &repo_id(owner, name), viewer_user_id).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(context)
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
    ) -> Result<
        (
            Option<scope_domain::repository::git::GitHead>,
            Vec<scope_domain::repository::git::GitPackSpan>,
        ),
        PostgresError,
    > {
        let state = self.repository_git_state(incarnation).await?;
        Ok((state.git_head, state.git_pack_spans))
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "repository_policy"))]
    pub async fn repository_policy(
        &self,
        context: &RepositoryAccessContext,
    ) -> Result<scope_domain::policy::Policy, PostgresError> {
        let tx = begin_metadata_read_snapshot(self.db.as_ref()).await?;
        let current = repository_access(&tx, &context.record.id, None)
            .await?
            .ok_or_else(|| PostgresError::not_found("repo not found"))?;
        ensure_current_context(context, &current)?;
        let policy = load_policy(&tx, &context.record.id).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(policy)
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
                    .await
                    .map_err(PostgresError::internal)?
            {
                tx.commit().await.map_err(PostgresError::internal)?;
                return Ok(Some(head.head_oid));
            }
            let metadata = super::projection_read_models::live_projection_read_model(
                &tx,
                &context.record.id,
                context.record.content_version,
                view,
            )
            .await?;
            tx.commit().await.map_err(PostgresError::internal)?;
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
    let Some(record) = load_repo_record(conn, repo_id).await? else {
        return Ok(None);
    };
    let access = match viewer_user_id {
        None => RepositoryAccess::public(),
        Some(user_id) => {
            let permissions = if user_id == record.owner_user_id {
                None
            } else {
                entities::repository_member::Entity::find_by_id((
                    repo_id.to_string(),
                    user_id.to_string(),
                ))
                .one(conn)
                .await
                .map_err(PostgresError::internal)?
                .map(entities::repository_member::Model::try_into_domain)
                .transpose()?
                .map(|member| member.permissions)
            };
            repository_access_for_user_id(
                &record.owner_user_id,
                record.lifecycle_state,
                permissions,
                user_id,
            )
        }
    };
    let views = super::projection_read_models::repository_views(conn, repo_id).await?;
    Ok(Some(RepositoryAccessContext {
        record,
        access,
        views,
    }))
}

async fn load_policy<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
) -> Result<scope_domain::policy::Policy, PostgresError> {
    let policy = entities::repository::Entity::find_by_id(repo_id)
        .select_only()
        .column(entities::repository::Column::Policy)
        .into_tuple::<serde_json::Value>()
        .one(conn)
        .await
        .map_err(PostgresError::internal)?
        .ok_or_else(|| PostgresError::not_found("repo not found"))?;
    serde_json::from_value(policy).map_err(PostgresError::internal)
}

pub(super) async fn load_repo_record<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
) -> Result<Option<RepoRecord>, PostgresError> {
    use entities::repository::{Column, Entity};
    let Some(row) = Entity::find()
        .select_only()
        .columns([
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
        ])
        .filter(Column::Id.eq(repo_id))
        .into_model::<AccessRow>()
        .one(conn)
        .await
        .map_err(PostgresError::internal)?
    else {
        return Ok(None);
    };
    Ok(Some(RepoRecord {
        id: row.id,
        incarnation_id: row.incarnation_id,
        owner_handle: row.owner_handle,
        name: row.name,
        owner_user_id: row.owner_user_id,
        description: row.description,
        website_url: row.website_url,
        lifecycle_state: entities::decode_enum(row.publication_state)?,
        change_version: integer_columns::i64_to_u64(
            row.change_version,
            "repository change version",
        )?,
        content_version: integer_columns::i64_to_u64(
            row.content_version,
            "repository content version",
        )?,
    }))
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
