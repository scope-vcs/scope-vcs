use super::{
    GeneratedIdKind, MergeRequestContentCommand, RequestStore, acquire_aggregate_lock,
    content_push_transactions::{RepositoryContentSnapshots, accept_and_persist_request_merge},
    entities,
    generated_ids::generate_id,
    repository_access::repository_access,
    request_access::ensure_user_exists,
    request_auto_merge::{
        StoredIntent, automatic_event_id, lock_active_intent_for_request,
        persist_existing_auto_merge_mutation,
    },
    request_lifecycle_effects::persist_lifecycle_mutation,
    request_revision_rows::latest_revision_for_request,
    request_rows::request_by_id,
};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseTransaction, EntityTrait, QueryFilter, QuerySelect,
    TransactionTrait,
};
use {
    crate::error::PostgresError,
    scope_domain::{
        repository::git::GitHead,
        requests::{
            MergeGateAuthority, MergeGateInput, MergeRequestInput, MergeRequestLanding,
            RequestAutoMergeReadiness, RequestAutoMergeStopReason, RequestLifecycleMutation,
            RequestMainPushCompletion, RequestViewer, evaluate_merge_gate,
            fulfill_request_auto_merge, lands_with_main, merge_request,
            request_auto_merge_readiness, request_checks_outcome, request_main_push_completion,
            stop_request_auto_merge,
        },
    },
};

#[derive(Clone, Debug)]
pub struct MergeRequestContentMutation {
    pub request: RequestLifecycleMutation,
    pub git_head: GitHead,
}

#[derive(Debug)]
pub struct MergeRequestContentsOutcome {
    pub landed: Vec<MergeRequestContentMutation>,
    pub rejected: Option<PostgresError>,
}

struct RejectedMerge {
    error: PostgresError,
    keeps_auto_merge_stop: bool,
}

impl<E> From<E> for RejectedMerge
where
    PostgresError: From<E>,
{
    fn from(error: E) -> Self {
        Self {
            error: error.into(),
            keeps_auto_merge_stop: false,
        }
    }
}

impl RejectedMerge {
    fn after_auto_merge_stop(error: PostgresError) -> Self {
        Self {
            error,
            keeps_auto_merge_stop: true,
        }
    }
}

impl RequestStore {
    pub async fn merge_request_content(
        &self,
        command: MergeRequestContentCommand,
    ) -> Result<MergeRequestContentMutation, PostgresError> {
        let mut outcome = self.merge_request_contents(vec![command]).await?;
        match outcome.rejected {
            Some(error) => Err(error),
            None => outcome
                .landed
                .pop()
                .ok_or_else(|| PostgresError::internal_message("merge landed nothing")),
        }
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "merge_request_contents", merges = commands.len()))]
    pub async fn merge_request_contents(
        &self,
        commands: Vec<MergeRequestContentCommand>,
    ) -> Result<MergeRequestContentsOutcome, PostgresError> {
        let Some(first) = commands.first() else {
            return Ok(MergeRequestContentsOutcome {
                landed: Vec::new(),
                rejected: None,
            });
        };
        let repo_id = scope_domain::repository::repo_id(&first.owner, &first.name);
        if commands.iter().any(|command| {
            scope_domain::repository::repo_id(&command.owner, &command.name) != repo_id
        }) {
            return Err(PostgresError::internal_message(
                "merged requests belong to different repositories",
            ));
        }
        let started = std::time::Instant::now();
        let commit_unix = first.now_unix;
        let tx = self.db.begin().await?;
        acquire_aggregate_lock(&tx, "repository", &repo_id).await?;
        let mut landed = Vec::with_capacity(commands.len());
        let mut landed_checks = Vec::with_capacity(commands.len());
        let mut rejected = None;
        let mut keeps_writes = false;
        for (index, command) in commands.into_iter().enumerate() {
            tx.execute_unprepared(&format!("SAVEPOINT merge_{index}"))
                .await?;
            match merge_one_request(&tx, &repo_id, command, started).await {
                Ok((mutation, checks)) => {
                    landed.push(mutation);
                    landed_checks.push(checks);
                    keeps_writes = true;
                }
                Err(rejection) => {
                    if rejection.keeps_auto_merge_stop {
                        keeps_writes = true;
                    } else {
                        tx.execute_unprepared(&format!("ROLLBACK TO SAVEPOINT merge_{index}"))
                            .await?;
                    }
                    rejected = Some(rejection.error);
                    break;
                }
            }
        }
        let committed_at = commit_unix.saturating_add(started.elapsed().as_secs());
        if let Some((expired, error)) =
            landed_checks
                .iter()
                .enumerate()
                .find_map(|(index, checks)| {
                    checks
                        .ensure_fresh(committed_at)
                        .err()
                        .map(|error| (index, error))
                })
        {
            tx.execute_unprepared(&format!("ROLLBACK TO SAVEPOINT merge_{expired}"))
                .await?;
            landed.truncate(expired);
            rejected = Some(error);
            keeps_writes = expired > 0;
        }
        if keeps_writes {
            tx.commit().await?;
        } else {
            tx.rollback().await?;
        }
        Ok(MergeRequestContentsOutcome { landed, rejected })
    }
}

async fn merge_one_request(
    tx: &DatabaseTransaction,
    repo_id: &str,
    command: MergeRequestContentCommand,
    started: std::time::Instant,
) -> Result<(MergeRequestContentMutation, evidence::LockedMergeChecks), RejectedMerge> {
    let MergeRequestContentCommand {
        stack_children,
        owner,
        name,
        request_id,
        actor_user_id,
        merged_event_id,
        expected_git_frontier,
        expected_repo_change_version,
        expected_request_head_oid,
        expected_auto_merge,
        expected_merge_evidence,
        update,
        landing_file_mutation,
        workflow_catalog,
        origin,
        now_unix,
    } = command;

    let request = request_by_id(tx, &request_id)
        .await?
        .filter(|request| request.repo_id == repo_id)
        .ok_or_else(|| PostgresError::not_found("request not found"))?;
    super::request_stack::lock_graph(tx, &request, None).await?;
    let locked_auto_merge = if let Some(expected) = &expected_auto_merge {
        let row = entities::request_auto_merge_intent::Entity::find_by_id(&expected.intent_id)
            .lock_exclusive()
            .one(tx)
            .await?
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
            .one(tx)
            .await?
            .map(StoredIntent::from_model)
            .transpose()?
    };
    let latest_revision = latest_revision_for_request(tx, &request.id).await?;
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
            persist_existing_auto_merge_mutation(tx, locked.model, &mutation).await?;
            return Err(RejectedMerge::after_auto_merge_stop(
                PostgresError::conflict("request changed since merge was prepared; retry merge"),
            ));
        }
        return Err(PostgresError::conflict(
            "request changed since merge was prepared; retry merge",
        )
        .into());
    }
    ensure_user_exists(tx, &actor_user_id).await?;

    let repo_row = entities::repository::Entity::find_by_id(repo_id)
        .one(tx)
        .await?
        .ok_or_else(|| PostgresError::not_found(format!("repo {owner}/{name} not found")))?;
    if locked_auto_merge
        .as_ref()
        .is_some_and(|locked| locked.intent.repository_incarnation_id != repo_row.incarnation_id)
    {
        return Err(PostgresError::conflict("auto-merge repository incarnation changed").into());
    }
    let context = repository_access(tx, repo_id, Some(&actor_user_id))
        .await?
        .ok_or_else(|| PostgresError::not_found(format!("repo {owner}/{name} not found")))?;
    let checks = evidence::locked_merge_checks(
        tx,
        &request,
        &expected_merge_evidence,
        now_unix.saturating_add(started.elapsed().as_secs()),
    )
    .await?;
    let viewer = RequestViewer::new(context.access.clone(), Some(&actor_user_id), false);
    let gate = evaluate_merge_gate(MergeGateInput::RequestMerge {
        repository_lifecycle: context.record.lifecycle_state,
        subject: (&request).into(),
        viewer: &viewer,
        views: &context.views,
        checks: request_checks_outcome(
            &request.id,
            &request.head_oid,
            checks.evaluation.as_ref(),
            &checks.results,
        ),
    });
    if expected_auto_merge.is_some()
        && let Some(locked) = &locked_auto_merge
    {
        if locked.intent.actor_user_id != actor_user_id {
            return Err(PostgresError::conflict(
                "auto-merge actor does not match the authorization",
            )
            .into());
        }
        if !context.access.is_maintainer() {
            let mutation = stop_request_auto_merge(
                &request,
                &locked.intent,
                RequestAutoMergeStopReason::AccessRevoked,
                automatic_event_id("stopped", &locked.intent.id),
                now_unix,
            )?;
            persist_existing_auto_merge_mutation(tx, locked.model.clone(), &mutation).await?;
            return Err(RejectedMerge::after_auto_merge_stop(
                PostgresError::permission_denied("repo maintainer required"),
            ));
        }
        match request_auto_merge_readiness(gate) {
            RequestAutoMergeReadiness::Ready => {}
            RequestAutoMergeReadiness::Waiting(_) => {
                return Err(PostgresError::conflict("auto-merge CI is not ready").into());
            }
            RequestAutoMergeReadiness::Stop(reason) => {
                let mutation = stop_request_auto_merge(
                    &request,
                    &locked.intent,
                    reason,
                    automatic_event_id("stopped", &locked.intent.id),
                    now_unix,
                )?;
                persist_existing_auto_merge_mutation(tx, locked.model.clone(), &mutation).await?;
                return Err(RejectedMerge::after_auto_merge_stop(
                    PostgresError::conflict(reason.message()),
                ));
            }
        }
    }
    if context.record.change_version != expected_repo_change_version {
        return Err(PostgresError::repository_moved(
            "repo changed since merge was prepared; retry merge",
        )
        .into());
    }
    let head = entities::git_head::Entity::find_by_id(repo_id)
        .one(tx)
        .await?
        .ok_or_else(|| PostgresError::conflict("repo has no accepted Git head"))?
        .try_into_domain()?;
    if head.frontier() != expected_git_frontier {
        return Err(PostgresError::repository_moved(
            "repo changed since merge was prepared; retry merge",
        )
        .into());
    }
    if gate.authority() != Some(MergeGateAuthority::RequestMerge) {
        let message = gate.reason().unwrap_or("request cannot be merged");
        return Err(if !context.access.is_maintainer() {
            PostgresError::permission_denied(message)
        } else {
            PostgresError::conflict(message)
        }
        .into());
    }
    let request_mutation = merge_request(
        &request,
        MergeRequestInput {
            request_id,
            actor_user_id,
            actor_is_maintainer: context.access.is_maintainer(),
            landing: MergeRequestLanding::NativeMerge,
            merged_head_oid: expected_request_head_oid,
            merged_main_oid: update.git_head.head_oid.clone(),
            merged_event_id,
            now_unix,
        },
    )?;

    let git_head = accept_and_persist_request_merge(
        tx,
        repo_row,
        update,
        RepositoryContentSnapshots {
            landing_file_mutation,
            workflow_catalog,
        },
        origin,
        now_unix,
    )
    .await?;

    persist_lifecycle_mutation(tx, &request_mutation.request, &request_mutation.events).await?;
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
        persist_existing_auto_merge_mutation(tx, locked.model, &fulfilled).await?;
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
    super::request_stack::retarget_children(
        tx,
        &request_mutation.request,
        &stack_children,
        now_unix,
    )
    .await?;
    checks.ensure_fresh(now_unix.saturating_add(started.elapsed().as_secs()))?;
    Ok((
        MergeRequestContentMutation {
            request: request_mutation,
            git_head,
        },
        checks,
    ))
}

mod evidence;

#[cfg(test)]
mod tests;

#[derive(Clone, Debug)]
pub struct LandedRequestCandidate {
    pub stack_children: Vec<super::RequestStackChildFacts>,
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
) -> Result<usize, PostgresError> {
    if completion.candidates.is_empty() {
        return Ok(0);
    }
    let repo = repository_access(tx, repo_id, Some(&completion.actor_user_id))
        .await?
        .ok_or_else(|| PostgresError::not_found("repo not found"))?;
    if evaluate_merge_gate(MergeGateInput::AuthorizedMainPush {
        repository_lifecycle: repo.record.lifecycle_state,
        access: &repo.access,
        views: &repo.views,
    })
    .authority()
        != Some(MergeGateAuthority::AuthorizedMainPush)
    {
        return Ok(0);
    }
    let mut ordered = Vec::with_capacity(completion.candidates.len());
    for candidate in completion.candidates {
        let depth = request_by_id(tx, &candidate.request_id)
            .await?
            .map(|request| request.stack_depth)
            .unwrap_or_default();
        ordered.push((depth, candidate.request_id.clone(), candidate));
    }
    ordered.sort_by(|left, right| (&left.0, &left.1).cmp(&(&right.0, &right.1)));
    let mut completed = 0;
    for (_, _, candidate) in ordered {
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
        super::request_stack::lock_graph(tx, &request, None).await?;
        if request_main_push_completion(&request, true)?
            == RequestMainPushCompletion::DeferUntilParentLands
        {
            continue;
        }
        let active_auto_merge = lock_active_intent_for_request(tx, &request.id).await?;
        let mutation = merge_request(
            &request,
            MergeRequestInput {
                request_id: candidate.request_id,
                actor_user_id: completion.actor_user_id.clone(),
                actor_is_maintainer: repo.access.is_maintainer(),
                landing: MergeRequestLanding::ContainingMain {
                    contains_request_head: true,
                },
                merged_head_oid: candidate.head_oid,
                merged_main_oid: main_oid.to_string(),
                merged_event_id: generate_id(GeneratedIdKind::RequestMergedEvent)?,
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
        super::request_stack::retarget_children(
            tx,
            &mutation.request,
            &candidate.stack_children,
            now_unix,
        )
        .await?;
        completed += 1;
    }
    Ok(completed)
}
