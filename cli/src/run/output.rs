use crate::display::short_oid;
use scope_api_contract::{
    AttemptState, CacheColdReason, CacheFinalState, CachePreparation, RepositoryRunAttemptResponse,
    RepositoryRunCacheResponse, RepositoryRunDetailResponse, RepositoryRunJobState, RunState,
};

pub(super) fn detail_lines(detail: &RepositoryRunDetailResponse) -> Vec<String> {
    let mut lines = vec![format!(
        "Run {} · {}",
        run_state_label(detail.run.state),
        short_oid(&detail.run.git_oid),
    )];
    lines.push("Jobs:".to_string());
    for job_detail in &detail.jobs {
        lines.push(format!(
            "  {} · {}",
            job_detail.job.key,
            job_state_label(job_detail.job.state),
        ));
        for attempt in &job_detail.attempts {
            lines.push(format!(
                "    {} · {}",
                attempt.id,
                attempt_state_label(attempt),
            ));
            lines.extend(environment_lines(
                Some(job_detail.job.pinned_container_image.as_str()),
                attempt,
            ));
        }
    }
    lines
}

fn environment_lines(
    pinned_image: Option<&str>,
    attempt: &RepositoryRunAttemptResponse,
) -> Vec<String> {
    if attempt.caches.is_empty() && pinned_image.is_none() && attempt.cache_setup.is_none() {
        return Vec::new();
    }
    let mut warm = 0;
    let mut cold = 0;
    let mut unavailable = 0;
    for cache in &attempt.caches {
        match cache.observation.as_ref().map(|fact| fact.preparation) {
            Some(CachePreparation::Exact | CachePreparation::Compatible) => warm += 1,
            Some(CachePreparation::Cold { .. }) => cold += 1,
            None => unavailable += 1,
        }
    }
    let mut summary = Vec::new();
    if warm > 0 {
        summary.push(format!("{warm} warm"));
    }
    if cold > 0 {
        summary.push(format!("{cold} cold"));
    }
    if unavailable > 0 {
        summary.push(format!("{unavailable} not reported"));
    }
    if let Some(cache_setup) = &attempt.cache_setup {
        summary.push(format!(
            "setup {} (authorization {})",
            duration_label(cache_setup.wall_ms),
            duration_label(cache_setup.authorization_ms),
        ));
    }
    if let Some(image) = pinned_image {
        summary.push(image_label(image));
    }
    let mut lines = vec![format!("      Environment · {}", summary.join(" · "))];
    lines.extend(attempt.caches.iter().map(cache_line));
    lines
}

fn cache_line(cache: &RepositoryRunCacheResponse) -> String {
    let Some(observation) = &cache.observation else {
        return format!("        {} · not reported", cache.name);
    };
    let preparation = match observation.preparation {
        CachePreparation::Exact => "exact".to_string(),
        CachePreparation::Compatible => "compatible".to_string(),
        CachePreparation::Cold { reason } => {
            format!("cold · {}", cold_reason_label(reason))
        }
    };
    let finalization = finalization_label(observation.final_state, observation.finalize_ms);
    format!(
        "        {} · {} · {} · prepared {}",
        cache.name,
        preparation,
        finalization,
        duration_label(observation.prepare_ms),
    )
}

fn finalization_label(state: CacheFinalState, finalize_ms: Option<u64>) -> String {
    let state = match state {
        CacheFinalState::Pending => "pending",
        CacheFinalState::Ready => "ready",
        CacheFinalState::Evicted => "evicted",
    };
    finalize_ms.map_or_else(
        || state.to_string(),
        |milliseconds| format!("{state} · finalized {}", duration_label(milliseconds)),
    )
}

fn cold_reason_label(reason: CacheColdReason) -> &'static str {
    match reason {
        CacheColdReason::MetadataMissing => "no reusable entry for this identity",
        CacheColdReason::MetadataInvalid => "cache metadata invalid",
        CacheColdReason::MetadataNotReady => "cached volume not ready",
    }
}

fn duration_label(milliseconds: u64) -> String {
    if milliseconds < 1_000 {
        return format!("{milliseconds}ms");
    }
    let tenths = milliseconds.saturating_add(50) / 100;
    format!("{}.{:01}s", tenths / 10, tenths % 10)
}

fn image_label(image: &str) -> String {
    let digest = image
        .rsplit_once("@sha256:")
        .map(|(_, digest)| digest)
        .unwrap_or(image);
    format!("image sha256:{}", digest.get(..12).unwrap_or(digest))
}

pub(super) fn run_state_label(state: RunState) -> &'static str {
    match state {
        RunState::Queued => "queued",
        RunState::Dispatching => "dispatching",
        RunState::Running => "running",
        RunState::Succeeded => "succeeded",
        RunState::Failed => "failed",
        RunState::Canceled => "canceled",
        RunState::Lost => "lost",
    }
}

fn job_state_label(state: RepositoryRunJobState) -> &'static str {
    match state {
        RepositoryRunJobState::Blocked => "blocked",
        RepositoryRunJobState::Queued => "queued",
        RepositoryRunJobState::Dispatching => "dispatching",
        RepositoryRunJobState::Running => "running",
        RepositoryRunJobState::Succeeded => "succeeded",
        RepositoryRunJobState::Failed => "failed",
        RepositoryRunJobState::Skipped => "skipped",
        RepositoryRunJobState::Canceled => "canceled",
        RepositoryRunJobState::Lost => "lost",
    }
}

fn attempt_state_label(attempt: &RepositoryRunAttemptResponse) -> &'static str {
    match attempt.state {
        AttemptState::Dispatching => "dispatching",
        AttemptState::Running => "running",
        AttemptState::Succeeded => "succeeded",
        AttemptState::Failed => "failed",
        AttemptState::Canceled => "canceled",
        AttemptState::Lost => "lost",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scope_api_contract::{
        RepositoryRunCacheObservationResponse, RepositoryRunCacheSetupObservationResponse,
        RepositoryRunStepResponse, RepositoryRunSummaryResponse,
    };

    #[test]
    fn detail_output_distinguishes_cold_from_missing_reports() {
        let detail = RepositoryRunDetailResponse {
            run: RepositoryRunSummaryResponse {
                id: "run-1".to_string(),
                workflow_name: "checks".to_string(),
                git_oid: "1234567890".to_string(),
                trigger: scope_api_contract::RepositoryRunTrigger::PushMain,
                state: RunState::Succeeded,
                cancellation_requested: false,
                created_at_unix: 1,
                updated_at_unix: 2,
                completed_at_unix: Some(2),
                can_cancel: false,
                can_retry: true,
            },
            jobs: vec![scope_api_contract::RepositoryRunJobDetailResponse {
                job: scope_api_contract::RepositoryRunJobResponse {
                    key: "backend".to_string(),
                    needs: vec![],
                    pinned_container_image: format!("registry/scope@sha256:{}", "a".repeat(64),),
                    state: RepositoryRunJobState::Succeeded,
                    created_at_unix: 1,
                    started_at_unix: Some(1),
                    updated_at_unix: 2,
                    completed_at_unix: Some(2),
                },
                attempts: vec![RepositoryRunAttemptResponse {
                    id: "attempt-1".to_string(),
                    number: 1,
                    external_run_id: Some("external-run-1".to_string()),
                    runtime_version: "0.1.0".to_string(),
                    state: AttemptState::Succeeded,
                    created_at_unix: 1,
                    started_at_unix: Some(1),
                    completed_at_unix: Some(2),
                    terminal_reason: None,
                    cache_setup: Some(RepositoryRunCacheSetupObservationResponse {
                        authorization_ms: 7,
                        wall_ms: 80,
                    }),
                    caches: vec![
                        RepositoryRunCacheResponse {
                            name: "cargo".to_string(),
                            path: "/cache/cargo".to_string(),
                            observation: Some(RepositoryRunCacheObservationResponse {
                                workflow_path: "/.scope/runs/checks.yml".to_string(),
                                job_key: "backend".to_string(),
                                identity_digest: "b".repeat(64),
                                preparation: CachePreparation::Cold {
                                    reason: CacheColdReason::MetadataMissing,
                                },
                                key_ms: 2,
                                metadata_ms: 10,
                                size_bytes: 0,
                                download_verify_ms: 0,
                                sync_ms: 0,
                                extraction_ms: 0,
                                prepare_ms: 12,
                                final_state: CacheFinalState::Ready,
                                finalize_ms: Some(8),
                            }),
                        },
                        RepositoryRunCacheResponse {
                            name: "target".to_string(),
                            path: "/cache/target".to_string(),
                            observation: None,
                        },
                        RepositoryRunCacheResponse {
                            name: "rustup".to_string(),
                            path: "/cache/rustup".to_string(),
                            observation: Some(RepositoryRunCacheObservationResponse {
                                workflow_path: "/.scope/runs/checks.yml".to_string(),
                                job_key: "backend".to_string(),
                                identity_digest: "c".repeat(64),
                                preparation: CachePreparation::Exact,
                                key_ms: 3,
                                metadata_ms: 4,
                                size_bytes: 12_582_912,
                                download_verify_ms: 20,
                                sync_ms: 5,
                                extraction_ms: 18,
                                prepare_ms: 50,
                                final_state: CacheFinalState::Ready,
                                finalize_ms: Some(4),
                            }),
                        },
                    ],
                    steps: Vec::<RepositoryRunStepResponse>::new(),
                }],
            }],
        };

        let output = detail_lines(&detail).join("\n");
        assert!(
            output.contains("1 warm · 1 cold · 1 not reported · setup 80ms (authorization 7ms)")
        );
        assert!(output.contains("no reusable entry for this identity"));
        assert!(output.contains("target · not reported"));
        assert!(!output.contains("identity changed"));
    }
}
