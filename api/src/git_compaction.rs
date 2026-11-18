mod pack;

use crate::{
    background_work::{BackgroundLoop, BackgroundWork, Cadence, Readiness, StopSignal},
    error::ApiError,
    state::AppState,
};
use pack::{CompactionPackMetrics, build_compacted_pack};
use scope_domain::repository::git::GitPoolPack;
use scope_git_process::ProcessError;
use scope_postgres::db::{GitCompactionClaim, MetadataStore};
use scope_service_runtime::unix_now;
use scope_storage::{
    ENCODING_VERSION, GitSegmentIngestTimings, GitSegmentReservation, GitSegmentStore,
    StagedGitSegment,
};
use std::time::{Duration, Instant};
use tracing::Instrument as _;

const GIT_COMPACTION_POLL_INTERVAL: Duration = Duration::from_secs(1);
const GIT_COMPACTION_TIMEOUT: Duration = Duration::from_secs(120);
const LEASE_SECONDS: u64 = GIT_COMPACTION_TIMEOUT.as_secs().saturating_add(30);

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CompactionOutcome {
    NoJob,
    Drained,
    Applied,
    Stale,
    Refused(String),
}

impl BackgroundWork {
    pub(crate) fn start_git_compaction(&mut self, concurrency: usize) {
        for _ in 0..concurrency {
            let stop = self.stopping();
            self.spawn(
                BackgroundLoop {
                    name: "Git compaction",
                    cadence: Cadence::AfterPass {
                        delay: GIT_COMPACTION_POLL_INTERVAL,
                        wakeup: None,
                    },
                    readiness: Readiness::SchemaRequired,
                },
                move |state| {
                    let stop = stop.clone();
                    async move { compact_until_idle(&state, &stop).await }
                },
            );
        }
    }
}

async fn compact_until_idle(state: &AppState, stop: &StopSignal) -> Result<(), ApiError> {
    while !stop.is_set() {
        match compact_one_git_repository(state).await {
            Ok(
                CompactionOutcome::Applied | CompactionOutcome::Stale | CompactionOutcome::Drained,
            ) => {}
            Ok(CompactionOutcome::NoJob) => return Ok(()),
            Ok(CompactionOutcome::Refused(reason)) => {
                tracing::warn!(reason, "Git compaction refused bounded replacement");
                return Ok(());
            }
            Err(error) => return Err(ApiError::internal_message(format!("{error:#}"))),
        }
    }
    Ok(())
}

pub(crate) async fn compact_one_git_repository(
    state: &AppState,
) -> anyhow::Result<CompactionOutcome> {
    let metadata = &state.metadata;
    let attempt_started = Instant::now();
    let claim_now_unix = unix_now()?;
    let candidate_started = Instant::now();
    let Some(claim) = metadata
        .jobs()
        .claim_git_compaction(
            state.repo_events.origin_id(),
            u64::try_from(
                state
                    .runtime_budgets
                    .git_storage_limits()
                    .max_object_bytes(),
            )
            .unwrap_or(u64::MAX),
            claim_now_unix,
            LEASE_SECONDS,
        )
        .await
        .map_err(|error| anyhow::anyhow!(error.message))?
    else {
        return Ok(CompactionOutcome::NoJob);
    };
    let span = tracing::info_span!(
        parent: None,
        "job.git_compaction",
        otel.kind = "consumer",
        scope.job.kind = "git_compaction",
        scope.job.id = claim.lease_generation(),
        scope.job.attempt = claim.attempts.saturating_add(1),
    );
    claim.queued_trace.link(&span);
    let started = ClaimTimings {
        attempt: attempt_started,
        candidate: candidate_started,
    };
    compact_claimed(state, claim, started)
        .instrument(span)
        .await
}

struct ClaimTimings {
    attempt: Instant,
    candidate: Instant,
}

async fn compact_claimed(
    state: &AppState,
    claim: GitCompactionClaim,
    started: ClaimTimings,
) -> anyhow::Result<CompactionOutcome> {
    let metadata = &state.metadata;
    let segment_store = &state.git_segment_store;
    let timeout = GIT_COMPACTION_TIMEOUT;
    let lease_seconds = LEASE_SECONDS;
    let attempt_started = started.attempt;
    let candidate_started = started.candidate;
    let Some(candidate) = claim.candidate.as_ref() else {
        metadata
            .jobs()
            .complete_git_compaction_claim(&claim, unix_now()?)
            .await
            .map_err(|error| anyhow::anyhow!(error.message))?;
        return Ok(CompactionOutcome::Drained);
    };
    let repository_id = candidate.incarnation.repository_id();
    let candidate_query_ms = elapsed_ms(candidate_started);
    let reservation = segment_store
        .reserve(repository_id)
        .map_err(anyhow::Error::new)?;
    metadata
        .repositories()
        .begin_git_segment_upload(
            repository_id,
            &reservation.segment_id,
            &reservation.object_key,
            ENCODING_VERSION,
            unix_now()?,
        )
        .await
        .map_err(|error| anyhow::anyhow!(error.message))?;
    let build_candidate = candidate.clone();
    let build_state = state.clone();
    let build_reservation = reservation.clone();
    let mut build = tokio::spawn(
        async move {
            build_replacement(&build_state, &build_candidate, build_reservation, timeout).await
        }
        .in_current_span(),
    );
    let renewal_interval = Duration::from_secs((lease_seconds / 3).max(1));
    let mut renewal = tokio::time::interval(renewal_interval);
    renewal.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    renewal.tick().await;
    let mut upload_heartbeat = tokio::time::interval(Duration::from_secs(60));
    upload_heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    upload_heartbeat.tick().await;
    let built = loop {
        tokio::select! {
            result = &mut build => {
                break match result {
                    Ok(result) => result,
                    Err(error) => Err(anyhow::anyhow!(
                        "Git compaction task failed: {error}"
                    )),
                };
            }
            _ = renewal.tick() => {
                match metadata.jobs().renew_git_compaction_claim(
                    &claim,
                    unix_now()?,
                    lease_seconds,
                ).await {
                    Ok(true) => {}
                    Ok(false) => tracing::warn!(
                        target_sequence = claim.target_sequence,
                        "Git compaction lease was lost while external work was still running"
                    ),
                    Err(error) => tracing::warn!(
                        error = %error.message,
                        target_sequence = claim.target_sequence,
                        "Git compaction lease renewal failed"
                    ),
                }
            }
            _ = upload_heartbeat.tick() => {
                if let Err(error) = metadata
                    .repositories()
                    .touch_git_segment_upload(&reservation.segment_id, unix_now()?)
                    .await
                {
                    tracing::warn!(
                        error = %error.message,
                        segment_id = reservation.segment_id,
                        "Git compaction upload heartbeat failed"
                    );
                }
            }
        }
    };
    let built = match built {
        Ok(compaction) => compaction,
        Err(failure) => {
            let failure_now_unix = unix_now()?;
            let attempt_remote_cleanup = !failure
                .downcast_ref::<ProcessError>()
                .is_some_and(ProcessError::is_timeout);
            if let Err(cleanup_error) = abandon_upload(
                metadata,
                segment_store,
                repository_id,
                &reservation,
                failure_now_unix,
                attempt_remote_cleanup,
            )
            .await
            {
                tracing::warn!(
                    error = %cleanup_error,
                    segment_id = reservation.segment_id,
                    "failed to discard unsuccessful Git compaction upload"
                );
            }
            if is_bounded_refusal(&failure) {
                metadata
                    .jobs()
                    .complete_git_compaction_claim(&claim, failure_now_unix)
                    .await
                    .map_err(|error| anyhow::anyhow!(error.message))?;
                return Ok(CompactionOutcome::Refused(failure.to_string()));
            }
            metadata
                .jobs()
                .fail_git_compaction_claim(&claim, &failure.to_string(), failure_now_unix)
                .await
                .map_err(|error| anyhow::anyhow!(error.message))?;
            return Err(failure);
        }
    };
    let final_renewal = metadata
        .jobs()
        .renew_git_compaction_claim(&claim, unix_now()?, lease_seconds)
        .await;
    if !matches!(final_renewal, Ok(true)) {
        let cleanup_now_unix = unix_now()?;
        if let Err(cleanup_error) = abandon_upload(
            metadata,
            segment_store,
            repository_id,
            &reservation,
            cleanup_now_unix,
            true,
        )
        .await
        {
            tracing::warn!(
                error = %cleanup_error,
                segment_id = reservation.segment_id,
                "failed to discard Git compaction after lease loss"
            );
        }
        if let Err(error) = final_renewal {
            return Err(anyhow::anyhow!(
                "renewing Git compaction lease before publication: {}",
                error.message
            ));
        }
        return Ok(CompactionOutcome::Stale);
    }
    if let Err(error) = metadata
        .repositories()
        .mark_git_segment_upload_ready(
            &built.staged.segment,
            built.staged.encrypted_bytes,
            unix_now()?,
        )
        .await
    {
        let cleanup_now_unix = unix_now()?;
        if let Err(cleanup_error) = abandon_upload(
            metadata,
            segment_store,
            repository_id,
            &reservation,
            cleanup_now_unix,
            true,
        )
        .await
        {
            tracing::warn!(
                error = %cleanup_error,
                segment_id = reservation.segment_id,
                "failed to discard Git compaction after ready transition failure"
            );
        }
        metadata
            .jobs()
            .fail_git_compaction_claim(&claim, &error.message, cleanup_now_unix)
            .await
            .map_err(|claim_error| anyhow::anyhow!(claim_error.message))?;
        return Err(anyhow::anyhow!(
            "marking Git compaction upload ready failed: {}",
            error.message
        ));
    }
    let persist_now_unix = unix_now()?;
    let persist_started = Instant::now();
    let persisted = metadata
        .jobs()
        .replace_main_pool_packs_with_compaction(
            repository_id,
            &candidate.plan,
            built.replacement,
            persist_now_unix,
        )
        .await;
    let persist_ms = elapsed_ms(persist_started);
    match persisted {
        Ok(applied) => {
            if applied {
                if let Err(error) = segment_store
                    .promote_verified_pack(&candidate.incarnation, &built.staged)
                    .await
                {
                    tracing::warn!(
                        error = %error,
                        segment_id = built.staged.segment.segment_id,
                        "failed to promote published Git compaction pack into the verified cache"
                    );
                    if let Err(cleanup_error) = segment_store.delete_local(&built.staged).await {
                        tracing::warn!(
                            error = %cleanup_error,
                            segment_id = built.staged.segment.segment_id,
                            "failed to remove published Git compaction staging pack"
                        );
                    }
                }
                cleanup_retired_local_segments(
                    segment_store,
                    repository_id,
                    candidate.plan.selected_packs(),
                )
                .await;
            } else if let Err(error) = discard_upload(
                metadata,
                segment_store,
                repository_id,
                &reservation,
                unix_now()?,
                true,
            )
            .await
            {
                tracing::warn!(
                    error = %error,
                    segment_id = reservation.segment_id,
                    "failed to delete stale Git compaction upload"
                );
            }
            metadata
                .jobs()
                .continue_git_compaction_claim(&claim, unix_now()?)
                .await
                .map_err(|error| anyhow::anyhow!(error.message))?;
            let outcome = if applied { "applied" } else { "stale" };
            log_compaction_attempt(
                &claim,
                candidate,
                &built.metrics,
                candidate_query_ms,
                persist_ms,
                elapsed_ms(attempt_started),
                outcome,
            );
            Ok(if applied {
                CompactionOutcome::Applied
            } else {
                CompactionOutcome::Stale
            })
        }
        Err(error) => {
            let failure_now_unix = unix_now()?;
            metadata
                .jobs()
                .fail_git_compaction_claim(&claim, &error.message, failure_now_unix)
                .await
                .map_err(|claim_error| anyhow::anyhow!(claim_error.message))?;
            if let Err(cleanup_error) = abandon_upload(
                metadata,
                segment_store,
                repository_id,
                &reservation,
                failure_now_unix,
                true,
            )
            .await
            {
                tracing::warn!(
                    error = %cleanup_error,
                    segment_id = reservation.segment_id,
                    "Git compaction publication failed and upload cleanup was deferred"
                );
            }
            Err(anyhow::anyhow!(
                "persisting Git compaction failed: {}",
                error.message
            ))
        }
    }
}

fn log_compaction_attempt(
    claim: &GitCompactionClaim,
    candidate: &scope_postgres::db::GitCompactionCandidate,
    metrics: &CompactionMetrics,
    candidate_query_ms: u64,
    persist_ms: u64,
    total_ms: u64,
    outcome: &str,
) {
    tracing::info!(
        outcome,
        repo_id = %candidate.incarnation.repository_id(),
        owner = %candidate.owner,
        repo = %candidate.name,
        target_sequence = claim.target_sequence,
        scheduler_attempts = claim.attempts,
        scheduler_queue_delay_ms = claim.queue_delay_ms,
        source_pack_count = metrics.pack.source_pack_count,
        source_pack_bytes = metrics.pack.source_pack_bytes,
        compacted_bytes = metrics.pack.compacted_bytes,
        inline_source_count = metrics.pack.inline_source_count,
        local_restore_count = metrics.pack.local_restore_count,
        remote_restore_count = metrics.pack.remote_restore_count,
        candidate_query_ms,
        init_ms = metrics.pack.init_ms,
        download_ms = metrics.pack.download_ms,
        index_ms = metrics.pack.index_ms,
        enumerate_ms = metrics.pack.enumerate_ms,
        pack_ms = metrics.pack.pack_ms,
        verify_ms = metrics.pack.verify_ms,
        pack_total_ms = metrics.pack.total_ms,
        local_write_ms = metrics
            .ingest
            .local_write
            .as_millis(),
        remote_upload_ms = metrics
            .ingest
            .remote_upload
            .as_millis(),
        encrypted_bytes = metrics.ingest.encrypted_bytes,
        uploaded_parts = metrics.ingest.uploaded_parts,
        ingest_chunk_bytes = metrics.ingest.chunk_bytes,
        ingest_channel_capacity = metrics.ingest.channel_capacity,
        persist_ms,
        total_ms,
        "Git compaction attempt completed"
    );
}

async fn abandon_upload(
    metadata: &MetadataStore,
    segment_store: &GitSegmentStore,
    repository_id: &str,
    reservation: &GitSegmentReservation,
    now_unix: u64,
    attempt_remote_cleanup: bool,
) -> anyhow::Result<()> {
    let can_delete = metadata
        .repositories()
        .abandon_git_segment_upload(&reservation.segment_id, now_unix)
        .await
        .map_err(|error| anyhow::anyhow!(error.message))?;
    discard_upload(
        metadata,
        segment_store,
        repository_id,
        reservation,
        now_unix,
        can_delete && attempt_remote_cleanup,
    )
    .await
}

async fn discard_upload(
    metadata: &MetadataStore,
    segment_store: &GitSegmentStore,
    repository_id: &str,
    reservation: &GitSegmentReservation,
    now_unix: u64,
    remote: bool,
) -> anyhow::Result<()> {
    let mut first_error = None;
    let mut record = |result: Result<(), anyhow::Error>| match result {
        Ok(()) => true,
        Err(error) => {
            first_error.get_or_insert(error);
            false
        }
    };
    let remote_deleted = remote
        && record(
            segment_store
                .cleanup_remote_bounded(&reservation.object_key)
                .await
                .map_err(anyhow::Error::new),
        );
    let local_deleted = record(
        segment_store
            .cleanup_local(repository_id, &reservation.segment_id)
            .await
            .map_err(anyhow::Error::new),
    );
    if remote_deleted && local_deleted {
        record(
            metadata
                .repositories()
                .mark_git_segment_upload_deleted(&reservation.segment_id, now_unix)
                .await
                .map_err(|error| anyhow::anyhow!(error.message)),
        );
    }
    first_error.map_or(Ok(()), Err)
}

struct BuiltCompaction {
    replacement: GitPoolPack,
    staged: StagedGitSegment,
    metrics: CompactionMetrics,
}

struct CompactionMetrics {
    pack: CompactionPackMetrics,
    ingest: GitSegmentIngestTimings,
}

async fn cleanup_retired_local_segments(
    segment_store: &GitSegmentStore,
    repository_id: &str,
    packs: &[GitPoolPack],
) {
    for segment in packs.iter().filter_map(|pack| pack.storage.segment()) {
        if let Err(error) = segment_store
            .cleanup_local(repository_id, &segment.segment_id)
            .await
        {
            tracing::warn!(
                error = %error,
                segment_id = segment.segment_id,
                "failed to remove retired local Git segment"
            );
        }
    }
}

async fn build_replacement(
    state: &AppState,
    candidate: &scope_postgres::db::GitCompactionCandidate,
    reservation: GitSegmentReservation,
    timeout: Duration,
) -> anyhow::Result<BuiltCompaction> {
    let pack = build_compacted_pack(
        state,
        &candidate.incarnation,
        &candidate.plan,
        reservation,
        timeout,
    )
    .await?;
    let replacement = candidate.plan.replacement(pack.staged.segment.clone())?;
    Ok(BuiltCompaction {
        replacement,
        metrics: CompactionMetrics {
            pack: pack.metrics,
            ingest: pack.staged.timings.clone(),
        },
        staged: pack.staged,
    })
}

fn is_bounded_refusal(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<ProcessError>()
        .is_some_and(|error| error.is_timeout() || error.is_stdout_limit())
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_timeout_and_output_limit_are_safe_refusals() {
        let timeout = anyhow::Error::new(ProcessError::TimedOut {
            action: "git fsck".to_string(),
            timeout_ms: 1,
            diagnostic: String::new(),
        });
        let oversized = anyhow::Error::new(ProcessError::StdoutLimitExceeded {
            action: "git pack-objects".to_string(),
            max_stdout_bytes: 4,
            diagnostic: String::new(),
        });

        assert!(is_bounded_refusal(&timeout));
        assert!(is_bounded_refusal(&oversized));
        assert!(!is_bounded_refusal(&anyhow::anyhow!("ordinary failure")));
    }
}
