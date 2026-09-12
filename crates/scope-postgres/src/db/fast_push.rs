use super::{
    GeneratedIdSource, RepositoryStore, acquire_aggregate_lock,
    content_push_transactions::{RepositoryContentSnapshots, accept_and_persist_content_push},
    entities,
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, TransactionTrait};
use {
    crate::error::PostgresError,
    scope_domain::{
        landing_file::RepositoryLandingFileMutation,
        repo_config::RepoConfig,
        repository::access::repository_push_policy_for_user_id,
        repository::{RepoLifecycleState, RepositoryIncarnation},
        reviewed_updates::content::{
            ReviewedUpdateAuthorization, ReviewedUpdateInput, authorize_reviewed_update,
        },
        runs::catalog::RepositoryWorkflowCatalog,
    },
};

pub struct ApplyContentOnlyPushCommand {
    pub incarnation: RepositoryIncarnation,
    pub owner: String,
    pub name: String,
    pub author_id: String,
    pub expected_git_frontier: scope_domain::repository::git::GitFrontier,
    pub update: ReviewedUpdateInput,
    pub landing_file_mutation: RepositoryLandingFileMutation,
    pub workflow_catalog: RepositoryWorkflowCatalog,
    pub push_trigger_input: scope_domain::runs::trigger::PushTriggerInput,
    pub now_unix: u64,
}

impl RepositoryStore {
    pub async fn apply_content_only_push(
        &self,
        command: ApplyContentOnlyPushCommand,
        generated_ids: &dyn GeneratedIdSource,
    ) -> Result<Option<scope_domain::repository::git::GitHead>, PostgresError> {
        let ApplyContentOnlyPushCommand {
            incarnation,
            owner,
            name,
            author_id,
            expected_git_frontier,
            update,
            landing_file_mutation,
            workflow_catalog,
            push_trigger_input,
            now_unix,
        } = command;
        let repo_id = scope_domain::repository::repo_id(&owner, &name);
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        acquire_aggregate_lock(&tx, "repository", &repo_id).await?;
        let repo_row = entities::repository::Entity::find_by_id(repo_id.clone())
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::not_found(format!("repo {owner}/{name} not found")))?;
        if repo_row.incarnation_id != incarnation.incarnation_id()
            || repo_row.id != incarnation.repository_id()
        {
            return Err(PostgresError::conflict(
                "repository was recreated since push preparation",
            ));
        }
        let publication_state: RepoLifecycleState =
            entities::decode_enum(repo_row.publication_state.clone())?;
        if publication_state != RepoLifecycleState::Ready {
            return Ok(None);
        }
        let head = entities::git_head::Entity::find_by_id(repo_id.clone())
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::conflict("repo has no accepted Git head"))?
            .try_into_domain()?;
        if head.frontier() != expected_git_frontier {
            return Err(PostgresError::conflict(
                "repo changed since push was reviewed; rerun scope push --main",
            ));
        }
        let member_permissions = entities::repository_member::Entity::find()
            .filter(entities::repository_member::Column::RepoId.eq(repo_id.clone()))
            .filter(entities::repository_member::Column::UserId.eq(author_id.clone()))
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .map(entities::repository_member::Model::try_into_domain)
            .transpose()?
            .map(|member| member.permissions);
        let push_policy = repository_push_policy_for_user_id(
            &repo_row.owner_user_id,
            publication_state,
            member_permissions,
            &author_id,
        );
        let current_config: RepoConfig = serde_json::from_value(repo_row.repo_config.clone())
            .map_err(PostgresError::internal)?;
        authorize_reviewed_update(ReviewedUpdateAuthorization {
            access: push_policy.access,
            push_mode: push_policy.mode,
            current_config: &current_config,
            proposed_config: &update.config,
        })?;
        let git_head = accept_and_persist_content_push(
            &tx,
            repo_row,
            update,
            RepositoryContentSnapshots {
                landing_file_mutation,
                workflow_catalog,
            },
            push_trigger_input,
            now_unix,
            generated_ids,
        )
        .await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(Some(git_head))
    }
}
