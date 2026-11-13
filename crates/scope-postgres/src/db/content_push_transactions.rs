use super::{
    dependency_analysis::enqueue_dependency_analysis_target,
    entities,
    git_compaction::schedule_git_compaction,
    git_heads::advance_git_head,
    git_segments::register_git_pool_pack,
    github_pushes::queue_main_publication,
    history_rows::{
        LiveFileRow, LiveFiles, file_rules, insert_commits, load_live_files_at, save_live_files,
    },
    integer_columns::{i64_to_u64, u64_to_i64},
    landing_files::apply_repository_landing_file_mutation,
    projection_read_models::fold_live_projection_read_models,
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseTransaction, EntityTrait,
    IntoActiveModel, QueryFilter, QueryOrder,
};
use std::collections::BTreeSet;
use {
    crate::error::PostgresError,
    scope_domain::{
        landing_file::RepositoryLandingFileMutation,
        policy::Policy,
        repo_actions::reviewed_update_domain_error,
        repo_config::RepoConfig,
        repository::git::GitHead,
        repository::updates::RequestMergeOrigin,
        reviewed_updates::content::{
            AcceptedContentPush, ContentPushState, ReviewedUpdateInput, accept_content_push,
            accept_request_merge,
        },
    },
};

pub(super) async fn accept_and_persist_content_push(
    tx: &DatabaseTransaction,
    repo_row: entities::repository::Model,
    update: ReviewedUpdateInput,
    landing_file_mutation: RepositoryLandingFileMutation,
    now_unix: u64,
) -> Result<PersistedMainChange, PostgresError> {
    accept_and_persist_content_update(
        tx,
        repo_row,
        update,
        landing_file_mutation,
        ContentUpdateKind::MainPush,
        now_unix,
    )
    .await
}

pub(super) async fn accept_and_persist_request_merge(
    tx: &DatabaseTransaction,
    repo_row: entities::repository::Model,
    update: ReviewedUpdateInput,
    landing_file_mutation: RepositoryLandingFileMutation,
    origin: RequestMergeOrigin,
    now_unix: u64,
) -> Result<PersistedMainChange, PostgresError> {
    accept_and_persist_content_update(
        tx,
        repo_row,
        update,
        landing_file_mutation,
        ContentUpdateKind::RequestMerge(origin),
        now_unix,
    )
    .await
}

pub(super) struct PersistedMainChange {
    pub(super) git_head: GitHead,
    pub(super) queued_main_publication: bool,
}

enum ContentUpdateKind {
    MainPush,
    RequestMerge(RequestMergeOrigin),
}

#[tracing::instrument(level = "debug", skip_all, fields(repository_id = %repo_row.id))]
async fn accept_and_persist_content_update(
    tx: &DatabaseTransaction,
    repo_row: entities::repository::Model,
    mut update: ReviewedUpdateInput,
    landing_file_mutation: RepositoryLandingFileMutation,
    kind: ContentUpdateKind,
    now_unix: u64,
) -> Result<PersistedMainChange, PostgresError> {
    let repo_id = repo_row.id.clone();
    let repo_incarnation_id = repo_row.incarnation_id.clone();
    let LiveFiles {
        files: live_files,
        rules: live_file_rules,
    } = load_live_files_at(
        tx,
        &repo_id,
        update.changes.iter().map(|change| &change.path),
    )
    .await?;
    let previous_commit = entities::logical_commit::Entity::find()
        .filter(entities::logical_commit::Column::RepoId.eq(&repo_id))
        .order_by_desc(entities::logical_commit::Column::Ordinal)
        .one(tx)
        .await?;
    let next_ordinal = previous_commit
        .as_ref()
        .map_or(0, |commit| commit.ordinal.saturating_add(1));
    let repo_config: RepoConfig =
        serde_json::from_value(repo_row.repo_config.clone()).map_err(PostgresError::internal)?;
    let stored_policy: Policy =
        serde_json::from_value(repo_row.policy.clone()).map_err(PostgresError::internal)?;
    let policy = stored_policy.clone().join_rules(live_file_rules);
    let mut live_paths = live_files.keys().cloned().collect::<BTreeSet<_>>();
    let change_version = i64_to_u64(repo_row.change_version, "repository change version")?;
    let content_version = i64_to_u64(repo_row.content_version, "repository content version")?;
    let previous_git_head = entities::git_head::Entity::find_by_id(&repo_id)
        .one(tx)
        .await?
        .map(entities::git_head::Model::try_into_domain)
        .transpose()?;
    update.previous_config = Some(repo_config.clone());
    let accepted = {
        let state = ContentPushState {
            change_version,
            content_version,
            policy,
            repo_config,
            live_files,
            git_head: previous_git_head.clone(),
        };
        match kind {
            ContentUpdateKind::MainPush => accept_content_push(state, update),
            ContentUpdateKind::RequestMerge(origin) => accept_request_merge(state, update, origin),
        }
    };
    let accepted = accepted.map_err(reviewed_update_domain_error)?;
    let AcceptedContentPush {
        change_version,
        content_version,
        policy,
        git_head,
        main_pack,
        logical_commit,
    } = accepted;

    for change in &logical_commit.changes {
        if change.new_content.is_some() {
            live_paths.insert(change.path.clone());
        } else {
            live_paths.remove(&change.path);
        }
    }
    let (repository_policy, live_file_rules) = policy.split_rules(|path| live_paths.contains(path));
    let file_rules = file_rules(&live_file_rules);

    let persisted_change_version = u64_to_i64(change_version, "repository change version")?;
    let mut repo_update = repo_row.into_active_model();
    repo_update.change_version = Set(persisted_change_version);
    repo_update.content_version = Set(u64_to_i64(content_version, "repository content version")?);
    if repository_policy != stored_policy {
        repo_update.policy =
            Set(serde_json::to_value(&repository_policy).map_err(PostgresError::internal)?);
    }
    repo_update.update(tx).await?;
    advance_git_head(tx, &repo_id, previous_git_head.as_ref(), &git_head).await?;
    register_git_pool_pack(tx, &repo_id, &main_pack, now_unix).await?;
    schedule_git_compaction(tx, &repo_id, git_head.push_sequence, now_unix).await?;
    let ordinal = usize::try_from(next_ordinal)
        .map_err(|_| PostgresError::internal_message("logical commit ordinal is invalid"))?;
    insert_commits(tx, &repo_id, ordinal, std::slice::from_ref(&logical_commit)).await?;
    save_live_files(
        tx,
        &repo_id,
        logical_commit.changes.iter().map(|change| {
            (
                &change.path,
                change.new_content.as_ref().map(|content| LiveFileRow {
                    content,
                    view: file_rules.get(&change.path).copied(),
                }),
            )
        }),
    )
    .await?;
    enqueue_dependency_analysis_target(
        tx,
        &scope_domain::repository::RepositoryIncarnation::new(&repo_id, repo_incarnation_id)
            .map_err(PostgresError::internal)?,
        content_version,
        &git_head.head_oid,
        scope_domain::dependency_analysis::DEPENDENCY_ANALYZER_VERSION,
        now_unix,
    )
    .await?;
    apply_repository_landing_file_mutation(tx, &repo_id, landing_file_mutation).await?;
    fold_live_projection_read_models(tx, &repo_id, content_version).await?;
    let queued_main_publication =
        queue_main_publication(tx, &repo_id, &git_head.head_oid, now_unix).await?;
    Ok(PersistedMainChange {
        git_head,
        queued_main_publication,
    })
}
