//! Atomic repository content merge plus request completion.

use super::{
    CompleteLandedRequestCommand, GeneratedIdSource, MergeRequestContentCommand, RequestStore,
    acquire_aggregate_lock,
    content_push_transactions::{RepositoryContentSnapshots, accept_and_persist_request_merge},
    entities,
    repository_access::repository_access,
    request_access::{ensure_user_exists, lock_request_repository},
    request_auto_merge::{
        StoredIntent, automatic_event_id, lock_active_intent_for_request,
        persist_existing_auto_merge_mutation, request_auto_merge_check_state,
    },
    request_revision_rows::latest_revision_for_request,
    request_rows::request_by_id,
    request_submission_transactions::persist_lifecycle_mutation,
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect, TransactionTrait};
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
                &checks.run_states,
            ) {
                RequestAutoMergeReadiness::Ready => {}
                RequestAutoMergeReadiness::Waiting(_) => {
                    return Err(PostgresError::conflict("auto-merge checks are not ready"));
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

        persist_lifecycle_mutation(&tx, &request_mutation).await?;
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

impl RequestStore {
    /// Records the merge of a request whose head a committed main push already carries.
    /// Returns `None` when the request moved or settled since the push was inspected.
    pub async fn complete_landed_request(
        &self,
        command: CompleteLandedRequestCommand,
    ) -> Result<Option<RequestLifecycleMutation>, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let (repo, request) =
            lock_request_repository(&tx, &command.request_id, &command.actor_user_id).await?;
        let active_auto_merge = lock_active_intent_for_request(&tx, &request.id).await?;
        if !lands_with_main(&request) || request.head_oid != command.landed_head_oid {
            return Ok(None);
        }
        let mut mutation = merge_request(
            &request,
            MergeRequestInput {
                request_id: command.request_id,
                actor_user_id: command.actor_user_id,
                actor_is_maintainer: repo.access.is_maintainer(),
                merged_head_oid: command.landed_head_oid,
                merged_main_oid: command.main_oid.clone(),
                merged_event_id: command.merged_event_id,
                now_unix: command.now_unix,
            },
        )?;
        persist_lifecycle_mutation(&tx, &mutation).await?;
        if let Some(stored) = active_auto_merge {
            let fulfilled = fulfill_request_auto_merge(
                &mutation.request,
                &stored.intent,
                command.main_oid,
                automatic_event_id("fulfilled", &stored.intent.id),
                command.now_unix,
            )?;
            persist_existing_auto_merge_mutation(&tx, stored.model, &fulfilled).await?;
            mutation.request = fulfilled.request;
            mutation.events.push(fulfilled.event);
        }
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(Some(mutation))
    }
}
