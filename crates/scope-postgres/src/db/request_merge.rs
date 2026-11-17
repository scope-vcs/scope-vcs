use super::{
    GeneratedIdKind, MergeRequestContentCommand, RequestStore, acquire_aggregate_lock,
    content_push_transactions::accept_and_persist_request_merge,
    entities,
    generated_ids::generate_id,
    repository_access::repository_access,
    request_auto_merge::{
        StoredIntent, automatic_event_id, lock_active_intent_for_request,
        persist_existing_auto_merge_mutation,
    },
    request_lifecycle_effects::persist_lifecycle_mutation,
    request_revision_rows::latest_revision_for_request,
    request_rows::request_by_id,
};
use sea_orm::{
    ConnectionTrait, DatabaseBackend, DatabaseTransaction, EntityTrait, QuerySelect, Statement,
    TransactionTrait,
};
use {
    crate::error::PostgresError,
    scope_domain::{
        repository::git::GitHead,
        requests::{
            ExpectedAutoMergeClaim, GitHubMergeLanding, MainAdvanceKind, MergeAutoMerge,
            MergeCommitFacts, MergeDecision, MergeFacts, MergeGitHead, MergeReadinessFacts,
            MergeRefusal, MergeRequestInput, MergeRequestLanding, MergeStep, RequestActor,
            RequestLifecycleMutation, RequestMainPushCompletion, RequestViewer,
            StoredAutoMergeClaim, decide_merge, fulfill_request_auto_merge, lands_with_main,
            merge_request, request_checks_outcome, request_main_push_completion,
        },
    },
};

#[derive(Clone, Debug)]
pub struct MergeRequestContentMutation {
    pub request: RequestLifecycleMutation,
    pub git_head: GitHead,
    pub queued_main_publication: bool,
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
        origin,
        now_unix,
    } = command;

    let request = request_by_id(tx, &request_id)
        .await?
        .filter(|request| request.repo_id == repo_id)
        .ok_or_else(|| PostgresError::not_found("request not found"))?;
    super::request_stack::lock_graph(tx, &request, None).await?;
    let locked_auto_merge = match &expected_auto_merge {
        Some(expected) => {
            entities::request_auto_merge_intent::Entity::find_by_id(&expected.intent_id)
                .lock_exclusive()
                .one(tx)
                .await?
                .map(StoredIntent::from_model)
                .transpose()?
        }
        None => lock_active_intent_for_request(tx, &request.id).await?,
    };
    let latest_revision = latest_revision_for_request(tx, &request.id).await?;
    let actor_exists = entities::user::Entity::find_by_id(&actor_user_id)
        .one(tx)
        .await?
        .is_some();
    let repo_row = entities::repository::Entity::find_by_id(repo_id)
        .one(tx)
        .await?
        .ok_or_else(|| PostgresError::not_found(format!("repo {owner}/{name} not found")))?;
    let context = repository_access(tx, repo_id, Some(&actor_user_id))
        .await?
        .ok_or_else(|| PostgresError::not_found(format!("repo {owner}/{name} not found")))?;
    let git_frontier = entities::git_head::Entity::find_by_id(repo_id)
        .one(tx)
        .await?
        .map(|head| head.try_into_domain())
        .transpose()?
        .map(|head| head.frontier());
    let checks = evidence::locked_merge_checks(
        tx,
        &request,
        &expected_merge_evidence,
        now_unix.saturating_add(started.elapsed().as_secs()),
    )
    .await;
    let readiness = match &checks {
        Ok(checks) => Some((
            super::projection_read_models::request_view_mains(
                tx,
                &request.repo_id,
                context.record.content_version,
            )
            .await?,
            request_checks_outcome(
                &request.id,
                &request.head_oid,
                checks.evaluation.as_ref(),
                &checks.results,
            ),
            super::github::review_bot_statuses::reviews_outcome(tx, &request, &context.views)
                .await?,
        )),
        Err(_) => None,
    };
    let decision = decide_merge(
        MergeFacts {
            request: &request,
            repository_id: repo_id,
            repository_lifecycle: context.record.lifecycle_state,
            actor: RequestActor {
                access: &context.access,
                is_invitee: false,
                views: &context.views,
            },
            actor_user_id: &actor_user_id,
        },
        MergeStep::Commit(Box::new(MergeCommitFacts {
            latest_revision: latest_revision.as_ref(),
            auto_merge: match (&expected_auto_merge, &locked_auto_merge) {
                (Some(expected), stored) => MergeAutoMerge::Claimed {
                    expected: ExpectedAutoMergeClaim {
                        revision_id: &expected.revision_id,
                        head_oid: &expected.head_oid,
                        claim_token: &expected.claim_token,
                        fulfilled_event_id: &expected.fulfilled_event_id,
                    },
                    stored: stored.as_ref().map(|stored| StoredAutoMergeClaim {
                        intent: &stored.intent,
                        claim_token: stored.model.claim_token.as_deref(),
                        claim_expires_at_unix: stored
                            .model
                            .claim_expires_at_unix
                            .and_then(|expires| u64::try_from(expires).ok()),
                    }),
                },
                (None, Some(stored)) => MergeAutoMerge::Active(&stored.intent),
                (None, None) => MergeAutoMerge::None,
            },
            actor_exists,
            repository_incarnation_id: &repo_row.incarnation_id,
            repository_change_version: context.record.change_version,
            git_head: MergeGitHead::observed(git_frontier.as_ref(), &expected_git_frontier),
            readiness: readiness.as_ref().map(|(view_mains, checks, reviews)| {
                MergeReadinessFacts {
                    view_mains,
                    checks: *checks,
                    reviews: *reviews,
                }
            }),
            prepared_request_head_oid: &expected_request_head_oid,
            prepared_repository_change_version: expected_repo_change_version,
            merged_main_oid: &update.git_head.head_oid,
            merged_event_id,
            now_unix,
        })),
    );
    let outcome = match decision {
        Ok(MergeDecision::Merge(outcome)) => *outcome,
        Ok(MergeDecision::Proceed) => {
            return Err(PostgresError::internal_message("merge commit was not decided").into());
        }
        Err(MergeRefusal::Rejected(error)) => return Err(PostgresError::from(error).into()),
        Err(MergeRefusal::RepositoryMoved) => {
            return Err(PostgresError::repository_moved(
                "repo changed since merge was prepared; retry merge",
            )
            .into());
        }
        Err(MergeRefusal::ReadinessUnavailable) => {
            return Err(checks
                .err()
                .unwrap_or_else(|| {
                    PostgresError::internal_message("merge readiness was not loaded")
                })
                .into());
        }
        Err(MergeRefusal::AutoMergeStopped { error, stop }) => {
            let locked = locked_auto_merge.ok_or_else(|| {
                PostgresError::internal_message("stopped an auto-merge that was not locked")
            })?;
            persist_existing_auto_merge_mutation(tx, locked.model, &stop).await?;
            return Err(RejectedMerge::after_auto_merge_stop(error.into()));
        }
    };
    let checks = checks?;

    let persisted = accept_and_persist_request_merge(
        tx,
        repo_row,
        update,
        landing_file_mutation,
        origin,
        now_unix,
    )
    .await?;
    let git_head = persisted.git_head;

    persist_lifecycle_mutation(tx, &outcome.request.request, &outcome.request.events).await?;
    record_main_advance(
        tx,
        repo_id,
        &actor_user_id,
        MainAdvanceKind::NativeMerge,
        now_unix,
    )
    .await?;
    let fulfilled = outcome.fulfill_auto_merge(&git_head.head_oid, now_unix)?;
    let request_mutation = match (fulfilled, locked_auto_merge) {
        (Some(fulfilled), Some(locked)) => {
            persist_existing_auto_merge_mutation(tx, locked.model, &fulfilled).await?;
            RequestLifecycleMutation {
                request: fulfilled.request,
                events: outcome
                    .request
                    .events
                    .into_iter()
                    .chain(std::iter::once(fulfilled.event))
                    .collect(),
            }
        }
        _ => outcome.request,
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
            queued_main_publication: persisted.queued_main_publication,
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
    pub main_contains: MainContains,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MainContains {
    RequestHead,
    GitHubMergeCommit(String),
}

#[derive(Clone, Debug)]
pub struct LandedRequestCompletion {
    pub actor_user_id: String,
    pub candidates: Vec<LandedRequestCandidate>,
}

pub(super) async fn complete_main_push(
    tx: &DatabaseTransaction,
    repo_id: &str,
    main_oid: &str,
    completion: LandedRequestCompletion,
    now_unix: u64,
) -> Result<usize, PostgresError> {
    let advanced_by_user_id = completion.actor_user_id.clone();
    let kind = MainAdvanceKind::Push;
    let completed =
        complete_landed_requests(tx, repo_id, main_oid, kind, completion, now_unix).await?;
    record_main_advance(tx, repo_id, &advanced_by_user_id, kind, now_unix).await?;
    Ok(completed)
}

async fn record_main_advance(
    tx: &DatabaseTransaction,
    repo_id: &str,
    advanced_by_user_id: &str,
    kind: MainAdvanceKind,
    now_unix: u64,
) -> Result<(), PostgresError> {
    tx.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "UPDATE scope_git_heads SET main_advanced_by_user_id = $2, main_advance_kind = $3 WHERE repo_id = $1",
        [repo_id.into(), advanced_by_user_id.into(), entities::encode_enum(kind)?.into()],
    ))
    .await?;
    super::github::pull_requests::requeue_merged_carriers_of_open_requests(tx, repo_id, now_unix)
        .await
}

pub(super) struct MainAdvance {
    pub main_oid: String,
    pub advanced_by_user_id: Option<String>,
    pub kind: MainAdvanceKind,
}

pub(super) async fn latest_main_advance(
    tx: &DatabaseTransaction,
    repo_id: &str,
) -> Result<Option<MainAdvance>, PostgresError> {
    tx.query_one_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "SELECT head_oid, main_advanced_by_user_id, main_advance_kind FROM scope_git_heads WHERE repo_id = $1",
        [repo_id.into()],
    ))
    .await?
    .map(|head| {
        Ok(MainAdvance {
            main_oid: head.try_get("", "head_oid")?,
            advanced_by_user_id: head.try_get("", "main_advanced_by_user_id")?,
            kind: entities::decode_enum(head.try_get("", "main_advance_kind")?)?,
        })
    })
    .transpose()
}

pub(super) async fn land_request_merged_on_github(
    tx: &DatabaseTransaction,
    request_id: &str,
    main: MainAdvance,
    merge_commit_oid: &str,
    stack_children: Vec<super::RequestStackChildFacts>,
    now_unix: u64,
) -> Result<bool, PostgresError> {
    let (Some(request), Some(actor_user_id)) = (
        request_by_id(tx, request_id).await?,
        main.advanced_by_user_id,
    ) else {
        return Ok(false);
    };
    let completed = complete_landed_requests(
        tx,
        &request.repo_id,
        &main.main_oid,
        main.kind,
        LandedRequestCompletion {
            actor_user_id,
            candidates: vec![LandedRequestCandidate {
                stack_children,
                request_id: request.id,
                head_oid: request.head_oid,
                main_contains: MainContains::GitHubMergeCommit(merge_commit_oid.to_string()),
            }],
        },
        now_unix,
    )
    .await?;
    Ok(completed == 1)
}

pub(super) async fn complete_landed_requests(
    tx: &DatabaseTransaction,
    repo_id: &str,
    main_oid: &str,
    advance_kind: MainAdvanceKind,
    completion: LandedRequestCompletion,
    now_unix: u64,
) -> Result<usize, PostgresError> {
    if completion.candidates.is_empty() {
        return Ok(0);
    }
    let repo = repository_access(tx, repo_id, Some(&completion.actor_user_id))
        .await?
        .ok_or_else(|| PostgresError::not_found("repo not found"))?;
    let viewer = RequestViewer::new(repo.access.clone(), Some(&completion.actor_user_id), false);
    let mut ordered = Vec::with_capacity(completion.candidates.len());
    for candidate in completion.candidates {
        let depth = request_by_id(tx, &candidate.request_id)
            .await?
            .map(|request| request.stack_depth)
            .unwrap_or_default();
        ordered.push((depth, candidate.request_id.clone(), candidate));
    }
    ordered.sort_by(|left, right| (&left.0, &left.1).cmp(&(&right.0, &right.1)));
    let connection = super::github::connections::repository_github_connection(tx, repo_id).await?;
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
        if advance_kind
            .landing_gate(repo.record.lifecycle_state, &request, &viewer, &repo.views)
            .authority()
            .is_none()
        {
            continue;
        }
        super::request_stack::lock_graph(tx, &request, None).await?;
        let pull_request;
        let landing = match &candidate.main_contains {
            MainContains::RequestHead => MergeRequestLanding::ContainingMain {
                contains_request_head: true,
            },
            MainContains::GitHubMergeCommit(commit_in_main) => {
                pull_request =
                    super::github::pull_requests::github_pull_request(tx, &request.id).await?;
                let merge = GitHubMergeLanding {
                    connection: connection.as_ref(),
                    pull_request: pull_request.as_ref(),
                    commit_in_main,
                };
                if !merge.lands(&request) {
                    continue;
                }
                MergeRequestLanding::GitHubMerge(merge)
            }
        };
        if request_main_push_completion(&request, landing)?
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
                landing,
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
