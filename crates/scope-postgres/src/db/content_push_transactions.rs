//! Shared repository-content persistence for transactions with additional domain effects.

use super::{
    GeneratedIdSource,
    dependency_analysis::enqueue_dependency_analysis_target,
    entities,
    git_compaction::schedule_git_compaction,
    git_segments::{load_git_pack_spans, publish_git_segment},
    history_rows::{insert_commits, save_live_files},
    integer_columns::{i64_to_u64, u64_to_i64},
    landing_files::apply_repository_landing_file_mutation,
    outbox::enqueue_projection_read_model_rebuild,
    push_triggers::enqueue_push_main_trigger_evaluation,
    workflow_catalogs::apply_repository_workflow_catalog,
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseTransaction, EntityTrait,
    IntoActiveModel, QueryFilter, QueryOrder,
};
use std::collections::BTreeMap;
use {
    crate::error::PostgresError,
    scope_domain::{
        landing_file::RepositoryLandingFileMutation,
        policy::{Policy, ScopePath},
        repo_actions::reviewed_update_domain_error,
        repo_config::RepoConfig,
        repo_control::REPO_RULES_PATH,
        repository::git::GitHead,
        repository::updates::RequestMergeOrigin,
        reviewed_updates::content::{
            AcceptedContentPush, ContentPushState, ReviewedUpdateInput, accept_content_push,
            accept_request_merge,
        },
        runs::catalog::RepositoryWorkflowCatalog,
        runs::trigger::PushTriggerInput,
    },
};

pub(super) async fn accept_and_persist_content_push(
    tx: &DatabaseTransaction,
    repo_row: entities::repository::Model,
    update: ReviewedUpdateInput,
    snapshots: RepositoryContentSnapshots,
    push_trigger_input: PushTriggerInput,
    now_unix: u64,
    generated_ids: &dyn GeneratedIdSource,
) -> Result<GitHead, PostgresError> {
    accept_and_persist_content_update(
        tx,
        repo_row,
        update,
        snapshots,
        ContentUpdateKind::MainPush(push_trigger_input),
        now_unix,
        generated_ids,
    )
    .await
}

pub(super) async fn accept_and_persist_request_merge(
    tx: &DatabaseTransaction,
    repo_row: entities::repository::Model,
    update: ReviewedUpdateInput,
    snapshots: RepositoryContentSnapshots,
    origin: RequestMergeOrigin,
    now_unix: u64,
    generated_ids: &dyn GeneratedIdSource,
) -> Result<GitHead, PostgresError> {
    accept_and_persist_content_update(
        tx,
        repo_row,
        update,
        snapshots,
        ContentUpdateKind::RequestMerge(origin),
        now_unix,
        generated_ids,
    )
    .await
}

enum ContentUpdateKind {
    MainPush(PushTriggerInput),
    RequestMerge(RequestMergeOrigin),
}

pub(super) struct RepositoryContentSnapshots {
    pub(super) landing_file_mutation: RepositoryLandingFileMutation,
    pub(super) workflow_catalog: RepositoryWorkflowCatalog,
}

/// One span covers the whole persistence step, so a subscriber that records
/// span timing sees its duration; the calling transaction logs lock, body and
/// commit timings itself.
#[tracing::instrument(level = "debug", skip_all, fields(repository_id = %repo_row.id))]
async fn accept_and_persist_content_update(
    tx: &DatabaseTransaction,
    repo_row: entities::repository::Model,
    mut update: ReviewedUpdateInput,
    snapshots: RepositoryContentSnapshots,
    kind: ContentUpdateKind,
    now_unix: u64,
    generated_ids: &dyn GeneratedIdSource,
) -> Result<GitHead, PostgresError> {
    let RepositoryContentSnapshots {
        landing_file_mutation,
        workflow_catalog,
    } = snapshots;
    let repo_id = repo_row.id.clone();
    let repo_incarnation_id = repo_row.incarnation_id.clone();
    let mut changed_paths = update
        .changes
        .iter()
        .map(|change| change.path.as_str().to_string())
        .collect::<Vec<_>>();
    if !changed_paths.iter().any(|path| path == REPO_RULES_PATH) {
        changed_paths.push(REPO_RULES_PATH.to_string());
    }
    let live_files = entities::live_file::Entity::find()
        .filter(entities::live_file::Column::RepoId.eq(&repo_id))
        .filter(entities::live_file::Column::Path.is_in(changed_paths))
        .all(tx)
        .await
        .map_err(PostgresError::internal)?
        .into_iter()
        .map(|row| {
            Ok((
                ScopePath::parse(row.path).map_err(PostgresError::internal)?,
                serde_json::from_value(row.content).map_err(PostgresError::internal)?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>, PostgresError>>()?;
    let previous_commit = entities::logical_commit::Entity::find()
        .filter(entities::logical_commit::Column::RepoId.eq(&repo_id))
        .order_by_desc(entities::logical_commit::Column::Ordinal)
        .one(tx)
        .await
        .map_err(PostgresError::internal)?;
    let next_ordinal = previous_commit
        .as_ref()
        .map_or(0, |commit| commit.ordinal.saturating_add(1));
    let repo_config: RepoConfig =
        serde_json::from_value(repo_row.repo_config.clone()).map_err(PostgresError::internal)?;
    let policy: Policy =
        serde_json::from_value(repo_row.policy.clone()).map_err(PostgresError::internal)?;
    let change_version = i64_to_u64(repo_row.change_version, "repository change version")?;
    let git_head = entities::git_head::Entity::find_by_id(&repo_id)
        .one(tx)
        .await
        .map_err(PostgresError::internal)?
        .map(entities::git_head::Model::try_into_domain)
        .transpose()?;
    update.previous_config = Some(repo_config.clone());
    let (accepted, push_trigger_input) = {
        let state = ContentPushState {
            change_version,
            policy,
            repo_config,
            live_files,
            git_head,
        };
        match kind {
            ContentUpdateKind::MainPush(input) => (accept_content_push(state, update), Some(input)),
            ContentUpdateKind::RequestMerge(origin) => {
                (accept_request_merge(state, update, origin), None)
            }
        }
    };
    let accepted = accepted.map_err(reviewed_update_domain_error)?;
    let AcceptedContentPush {
        change_version,
        policy,
        git_head,
        git_pack_span,
        logical_commit,
    } = accepted;
    let workflow_catalog = workflow_catalog
        .rebind_source_change_version(&repo_id, &git_head.head_oid, git_head.change_version)
        .map_err(PostgresError::internal)?;

    let persisted_change_version = u64_to_i64(change_version, "repository change version")?;
    let mut repo_update = repo_row.into_active_model();
    repo_update.change_version = Set(persisted_change_version);
    repo_update.policy = Set(serde_json::to_value(&policy).map_err(PostgresError::internal)?);
    repo_update
        .update(tx)
        .await
        .map_err(PostgresError::internal)?;
    entities::git_head::Entity::delete_by_id(&repo_id)
        .exec(tx)
        .await
        .map_err(PostgresError::internal)?;
    entities::git_head::Model::from_domain(&repo_id, &git_head)?
        .into_active_model()
        .insert(tx)
        .await
        .map_err(PostgresError::internal)?;
    entities::git_pack_span::Model::from_domain(&repo_id, &git_pack_span)?
        .into_active_model()
        .insert(tx)
        .await
        .map_err(PostgresError::internal)?;
    publish_git_segment(tx, &repo_id, &git_pack_span.segment, now_unix).await?;
    schedule_git_compaction(tx, &repo_id, git_head.push_sequence, now_unix).await?;
    let pinned_pack_spans = load_git_pack_spans(tx, &repo_id).await?;
    let ordinal = usize::try_from(next_ordinal)
        .map_err(|_| PostgresError::internal_message("logical commit ordinal is invalid"))?;
    insert_commits(tx, &repo_id, ordinal, std::slice::from_ref(&logical_commit)).await?;
    save_live_files(
        tx,
        &repo_id,
        logical_commit
            .changes
            .iter()
            .map(|change| (&change.path, change.new_content.as_ref())),
    )
    .await?;
    enqueue_dependency_analysis_target(
        tx,
        &scope_domain::repository::RepositoryIncarnation::new(&repo_id, repo_incarnation_id)
            .map_err(PostgresError::internal)?,
        change_version,
        &git_head.head_oid,
        scope_domain::dependency_analysis::DEPENDENCY_ANALYZER_VERSION,
        now_unix,
    )
    .await?;
    apply_repository_landing_file_mutation(tx, &repo_id, landing_file_mutation).await?;
    apply_repository_workflow_catalog(tx, &workflow_catalog).await?;
    enqueue_projection_read_model_rebuild(tx, &repo_id, change_version, now_unix, generated_ids)
        .await?;
    if let Some(input) = push_trigger_input {
        enqueue_push_main_trigger_evaluation(
            tx,
            &repo_id,
            &git_head,
            &pinned_pack_spans,
            &input,
            now_unix,
            generated_ids,
        )
        .await?;
    }
    Ok(git_head)
}
