use super::request_rows::{
    delete_request_rows, insert_request_event_row, request_event_by_id,
    save_request_branch_and_author, save_request_row,
};
use crate::error::PostgresError;
use scope_domain::requests::{
    DELETED_DRAFT_EFFECTS, DISCARDED_CHANGE_EFFECTS, DeletedDraftEffect, DiscardedChangeEffect,
    EndedRequestEffect, Request, RequestCarrierFacts, RequestEvent, RequestTimelineMutation,
    ended_request_effects,
};
use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseTransaction, Statement};

pub(super) async fn persist_lifecycle_mutation(
    tx: &DatabaseTransaction,
    request: &Request,
    events: &[RequestEvent],
) -> Result<(), PostgresError> {
    for event in events {
        if request_event_by_id(tx, &event.id).await?.is_some() {
            return Err(PostgresError::conflict("request event already exists"));
        }
    }
    save_request_row(tx, request).await?;
    if request.is_terminal() {
        persist_ended_request_effects(tx, request).await?;
    }
    for event in events {
        insert_request_event_row(tx, event).await?;
    }
    Ok(())
}

async fn persist_ended_request_effects(
    tx: &DatabaseTransaction,
    request: &Request,
) -> Result<(), PostgresError> {
    let child_carriers = super::github::pull_requests::lock_carrier_and_child_carriers(
        tx,
        &request.repo_id,
        &request.id,
    )
    .await?;
    let connection =
        super::github::connections::repository_github_connection(tx, &request.repo_id).await?;
    let carrier = super::github::pull_requests::github_pull_request(tx, &request.id).await?;
    let now_unix = request.updated_at_unix;
    for effect in ended_request_effects(
        request,
        RequestCarrierFacts {
            connection: connection.as_ref(),
            pull_request: carrier.as_ref(),
        },
    ) {
        match effect {
            EndedRequestEffect::PurgeInvitees => {
                super::request_invitees::delete_request_invitees(tx, &request.id).await?;
            }
            EndedRequestEffect::DeleteGitHubBranch => {
                super::github::pushes::queue_github_branch_deletion(
                    tx,
                    &request.repo_id,
                    &request.id,
                    now_unix,
                )
                .await?;
            }
            EndedRequestEffect::CloseCarrierPullRequest => {
                super::github::pull_requests::queue_carrier_close(tx, request).await?;
            }
            EndedRequestEffect::RecomputeChildCarrierBases => {
                child_carriers.recompute_bases(tx, now_unix).await?;
            }
        }
    }
    Ok(())
}

pub(super) async fn persist_deleted_draft<C: sea_orm::ConnectionTrait>(
    tx: &C,
    request: &Request,
    now_unix: u64,
) -> Result<(), PostgresError> {
    for effect in DELETED_DRAFT_EFFECTS {
        match effect {
            DeletedDraftEffect::RetirePoolPacks => {
                super::git_segments::retire_request_pool_packs(tx, &request.id, now_unix).await?;
            }
            DeletedDraftEffect::DeleteGitHubBranch => {
                super::github::pushes::queue_github_branch_deletion(
                    tx,
                    &request.repo_id,
                    &request.id,
                    now_unix,
                )
                .await?;
            }
            DeletedDraftEffect::TombstoneAttachments => {
                super::request_media::tombstone_request_attachments(tx, &request.id, now_unix)
                    .await?;
            }
            DeletedDraftEffect::DeleteRequestRows => {
                delete_request_rows(tx, &request.id).await?;
            }
        }
    }
    Ok(())
}

pub(super) async fn persist_discarded_change<C: ConnectionTrait>(
    tx: &C,
    mutation: &RequestTimelineMutation,
    now_unix: u64,
) -> Result<(), PostgresError> {
    let request = &mutation.request;
    for effect in DISCARDED_CHANGE_EFFECTS {
        match effect {
            DiscardedChangeEffect::RetirePoolPacks => {
                super::git_segments::retire_request_pool_packs(tx, &request.id, now_unix).await?;
            }
            DiscardedChangeEffect::DeleteGitHubBranch => {
                super::github::pushes::queue_github_branch_deletion(
                    tx,
                    &request.repo_id,
                    &request.id,
                    now_unix,
                )
                .await?;
            }
            DiscardedChangeEffect::PurgeInvitees => {
                super::request_invitees::delete_request_invitees(tx, &request.id).await?;
            }
            DiscardedChangeEffect::DeleteChangeRows => {
                tx.execute_raw(Statement::from_sql_and_values(
                    DatabaseBackend::Postgres,
                    "WITH packs AS (DELETE FROM scope_git_pool_packs WHERE request_id = $1),
                          checks AS (
                              DELETE FROM scope_request_check_evaluations WHERE request_id = $1
                          ),
                          carriers AS (
                              DELETE FROM scope_github_pull_requests WHERE request_id = $1
                          ),
                          events AS (
                              DELETE FROM scope_request_events
                               WHERE request_id = $1
                                 AND kind IN ('RevisionPushed', 'BaseRetargeted')
                          )
                     DELETE FROM scope_request_revisions WHERE request_id = $1",
                    [request.id.clone().into()],
                ))
                .await?;
            }
        }
    }
    save_request_branch_and_author(tx, request).await?;
    insert_request_event_row(tx, &mutation.event).await
}
