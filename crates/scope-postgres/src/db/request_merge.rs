use super::{
    GeneratedIdKind, GeneratedIdSource, MergeRequestContentCommand, RequestStore,
    acquire_aggregate_lock,
    content_push_transactions::{RepositoryContentSnapshots, accept_and_persist_request_merge},
    entities,
    generated_ids::generate_id,
    repository_access::repository_access,
    request_access::ensure_user_exists,
    request_auto_merge::{
        StoredIntent, automatic_event_id, lock_active_intent_for_request,
        persist_existing_auto_merge_mutation, request_auto_merge_check_state,
    },
    request_lifecycle_effects::persist_lifecycle_mutation,
    request_revision_rows::latest_revision_for_request,
    request_rows::request_by_id,
};
use sea_orm::{
    ColumnTrait, DatabaseTransaction, EntityTrait, QueryFilter, QuerySelect, TransactionTrait,
};
use {
    crate::error::PostgresError,
    scope_domain::{
        repository::RepoLifecycleState,
        repository::git::GitHead,
        requests::{
            MergeRequestInput, RequestAutoMergeReadiness, RequestAutoMergeStopReason,
            RequestLifecycleMutation, fulfill_request_auto_merge, lands_with_main, merge_request,
            request_auto_merge_readiness, stop_request_auto_merge,
        },
    },
};

#[derive(Clone, Debug)]
pub struct MergeRequestContentMutation {
    pub request: RequestLifecycleMutation,
    pub git_head: GitHead,
}

impl RequestStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "merge_request_content"))]
    pub async fn merge_request_content(
        &self,
        command: MergeRequestContentCommand,
        generated_ids: &dyn GeneratedIdSource,
    ) -> Result<MergeRequestContentMutation, PostgresError> {
        let MergeRequestContentCommand {
            owner,
            name,
            request_id,
            actor_user_id,
            merged_event_id,
            expected_git_frontier,
            expected_repo_change_version,
            expected_request_head_oid,
            expected_auto_merge,
            update,
            landing_file_mutation,
            workflow_catalog,
            origin,
            now_unix,
        } = command;
        let repo_id = scope_domain::repository::repo_id(&owner, &name);
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        acquire_aggregate_lock(&tx, "repository", &repo_id).await?;
        acquire_aggregate_lock(&tx, "request", &request_id).await?;

        let request = request_by_id(&tx, &request_id)
            .await?
            .filter(|request| request.repo_id == repo_id)
            .ok_or_else(|| PostgresError::not_found("request not found"))?;
        let locked_auto_merge = if let Some(expected) = &expected_auto_merge {
            let row = entities::request_auto_merge_intent::Entity::find_by_id(&expected.intent_id)
                .lock_exclusive()
                .one(&tx)
                .await
                .map_err(PostgresError::internal)?
                .filter(|row| {
                    row.status == "Active"
                        && row.request_id == request.id
                        && row.repo_id == repo_id
                        && row.revision_id == expected.revision_id
                        && row.head_oid == expected.head_oid
                        && row.claim_token.as_deref() == Some(expected.claim_token.as_str())
                        && row
                            .claim_expires_at_unix
                            .is_some_and(|expires| expires >= now_unix as i64)
                })
                .ok_or_else(|| PostgresError::conflict("auto-merge claim is no longer current"))?;
            Some(StoredIntent::from_model(row)?)
        } else {
            entities::request_auto_merge_intent::Entity::find()
                .filter(entities::request_auto_merge_intent::Column::RequestId.eq(&request.id))
                .filter(entities::request_auto_merge_intent::Column::Status.eq("Active"))
                .lock_exclusive()
                .one(&tx)
                .await
                .map_err(PostgresError::internal)?
                .map(StoredIntent::from_model)
                .transpose()?
        };
        let latest_revision = latest_revision_for_request(&tx, &request.id).await?;
        let authorized_revision_is_current = locked_auto_merge.as_ref().is_none_or(|locked| {
            latest_revision.as_ref().is_some_and(|revision| {
                revision.id == locked.intent.revision_id
                    && revision.new_head_oid == locked.intent.head_oid
                    && request.head_oid == locked.intent.head_oid
            })
        });
        if request.head_oid != expected_request_head_oid || !authorized_revision_is_current {
            let should_stop_auto_merge = expected_auto_merge.is_some()
                || locked_auto_merge
                    .as_ref()
                    .is_some_and(|_| !authorized_revision_is_current);
            if should_stop_auto_merge && let Some(locked) = locked_auto_merge {
                let mutation = stop_request_auto_merge(
                    &request,
                    &locked.intent,
                    RequestAutoMergeStopReason::RequestChanged,
                    automatic_event_id("stopped", &locked.intent.id),
                    now_unix,
                )?;
                persist_existing_auto_merge_mutation(&tx, locked.model, &mutation).await?;
                tx.commit().await.map_err(PostgresError::internal)?;
            }
            return Err(PostgresError::conflict(
                "request changed since merge was prepared; retry merge",
            ));
        }
        ensure_user_exists(&tx, &actor_user_id).await?;

        let repo_row = entities::repository::Entity::find_by_id(repo_id.clone())
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::not_found(format!("repo {owner}/{name} not found")))?;
        if locked_auto_merge.as_ref().is_some_and(|locked| {
            locked.intent.repository_incarnation_id != repo_row.incarnation_id
        }) {
            return Err(PostgresError::conflict(
                "auto-merge repository incarnation changed",
            ));
        }
        let context = repository_access(&tx, &repo_id, Some(&actor_user_id))
            .await?
            .ok_or_else(|| PostgresError::not_found(format!("repo {owner}/{name} not found")))?;
        if expected_auto_merge.is_some()
            && let Some(locked) = &locked_auto_merge
        {
            if locked.intent.actor_user_id != actor_user_id {
                return Err(PostgresError::conflict(
                    "auto-merge actor does not match the authorization",
                ));
            }
            if !context.access.is_maintainer() {
                let mutation = stop_request_auto_merge(
                    &request,
                    &locked.intent,
                    RequestAutoMergeStopReason::AccessRevoked,
                    automatic_event_id("stopped", &locked.intent.id),
                    now_unix,
                )?;
                persist_existing_auto_merge_mutation(&tx, locked.model.clone(), &mutation).await?;
                tx.commit().await.map_err(PostgresError::internal)?;
                return Err(PostgresError::permission_denied("repo maintainer required"));
            }
            let checks = request_auto_merge_check_state(&tx, &locked.intent).await?;
            match request_auto_merge_readiness(
                &request.id,
                &request.head_oid,
                checks.evaluation.as_ref(),
                &checks.results,
            ) {
                RequestAutoMergeReadiness::Ready => {}
                RequestAutoMergeReadiness::Waiting(_) => {
                    return Err(PostgresError::conflict("auto-merge CI is not ready"));
                }
                RequestAutoMergeReadiness::Stop(reason) => {
                    let mutation = stop_request_auto_merge(
                        &request,
                        &locked.intent,
                        reason,
                        automatic_event_id("stopped", &locked.intent.id),
                        now_unix,
                    )?;
                    persist_existing_auto_merge_mutation(&tx, locked.model.clone(), &mutation)
                        .await?;
                    tx.commit().await.map_err(PostgresError::internal)?;
                    return Err(PostgresError::conflict(reason.message()));
                }
            }
        }
        if context.record.change_version != expected_repo_change_version {
            return Err(PostgresError::conflict(
                "repo changed since merge was prepared; retry merge",
            ));
        }
        if context.record.lifecycle_state != RepoLifecycleState::Ready {
            return Err(PostgresError::conflict("repo must be ready before merge"));
        }
        let head = entities::git_head::Entity::find_by_id(repo_id.clone())
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::conflict("repo has no accepted Git head"))?
            .try_into_domain()?;
        if head.frontier() != expected_git_frontier {
            return Err(PostgresError::conflict(
                "repo changed since merge was prepared; retry merge",
            ));
        }
        let request_mutation = merge_request(
            &request,
            MergeRequestInput {
                request_id,
                actor_user_id,
                actor_is_maintainer: context.access.is_maintainer(),
                merged_head_oid: expected_request_head_oid,
                merged_main_oid: update.git_head.head_oid.clone(),
                merged_event_id,
                now_unix,
            },
        )?;

        let git_head = accept_and_persist_request_merge(
            &tx,
            repo_row,
            update,
            RepositoryContentSnapshots {
                landing_file_mutation,
                workflow_catalog,
            },
            origin,
            now_unix,
            generated_ids,
        )
        .await?;

        persist_lifecycle_mutation(&tx, &request_mutation.request, &request_mutation.events)
            .await?;
        let request_mutation = if let Some(locked) = locked_auto_merge {
            let fulfilled_event_id = expected_auto_merge
                .as_ref()
                .map(|expected| expected.fulfilled_event_id.clone())
                .unwrap_or_else(|| automatic_event_id("fulfilled", &locked.intent.id));
            let fulfilled = fulfill_request_auto_merge(
                &request_mutation.request,
                &locked.intent,
                git_head.head_oid.clone(),
                fulfilled_event_id,
                now_unix,
            )?;
            persist_existing_auto_merge_mutation(&tx, locked.model, &fulfilled).await?;
            RequestLifecycleMutation {
                request: fulfilled.request,
                events: request_mutation
                    .events
                    .into_iter()
                    .chain(std::iter::once(fulfilled.event))
                    .collect(),
            }
        } else {
            request_mutation
        };
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(MergeRequestContentMutation {
            request: request_mutation,
            git_head,
        })
    }
}

#[cfg(test)]
mod tests;

#[derive(Clone, Debug)]
pub struct LandedRequestCandidate {
    pub request_id: String,
    pub head_oid: String,
}

#[derive(Clone, Debug)]
pub struct LandedRequestCompletion {
    pub actor_user_id: String,
    pub candidates: Vec<LandedRequestCandidate>,
}

pub(super) async fn complete_landed_requests(
    tx: &DatabaseTransaction,
    repo_id: &str,
    main_oid: &str,
    completion: LandedRequestCompletion,
    now_unix: u64,
    generated_ids: &dyn GeneratedIdSource,
) -> Result<usize, PostgresError> {
    if completion.candidates.is_empty() {
        return Ok(0);
    }
    let repo = repository_access(tx, repo_id, Some(&completion.actor_user_id))
        .await?
        .ok_or_else(|| PostgresError::not_found("repo not found"))?;
    if !repo.access.is_maintainer() || !repo.access.reads_full_view(&repo.views) {
        return Ok(0);
    }
    let mut completed = 0;
    for candidate in completion.candidates {
        acquire_aggregate_lock(tx, "request", &candidate.request_id).await?;
        let Some(request) = request_by_id(tx, &candidate.request_id)
            .await?
            .filter(|request| {
                request.repo_id == repo_id
                    && lands_with_main(request)
                    && request.head_oid == candidate.head_oid
            })
        else {
            continue;
        };
        let active_auto_merge = lock_active_intent_for_request(tx, &request.id).await?;
        let mutation = merge_request(
            &request,
            MergeRequestInput {
                request_id: candidate.request_id,
                actor_user_id: completion.actor_user_id.clone(),
                actor_is_maintainer: repo.access.is_maintainer(),
                merged_head_oid: candidate.head_oid,
                merged_main_oid: main_oid.to_string(),
                merged_event_id: generate_id(generated_ids, GeneratedIdKind::RequestMergedEvent)?,
                now_unix,
            },
        )?;
        persist_lifecycle_mutation(tx, &mutation.request, &mutation.events).await?;
        if let Some(stored) = active_auto_merge {
            let fulfilled = fulfill_request_auto_merge(
                &mutation.request,
                &stored.intent,
                main_oid.to_string(),
                automatic_event_id("fulfilled", &stored.intent.id),
                now_unix,
            )?;
            persist_existing_auto_merge_mutation(tx, stored.model, &fulfilled).await?;
        }
        completed += 1;
    }
    Ok(completed)
}
