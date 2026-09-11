use scope_api_contract::{
    RepositoryRunAttemptResponse, RepositoryRunCacheColdReason, RepositoryRunCacheFinalState,
    RepositoryRunCachePreparation, RepositoryRunCacheResponse, RepositoryRunDetailResponse,
    RepositoryRunJobState,
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
            Some(
                RepositoryRunCachePreparation::Exact | RepositoryRunCachePreparation::Compatible,
            ) => warm += 1,
            Some(RepositoryRunCachePreparation::Cold { .. }) => cold += 1,
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
        RepositoryRunCachePreparation::Exact => "exact".to_string(),
        RepositoryRunCachePreparation::Compatible => "compatible".to_string(),
        RepositoryRunCachePreparation::Cold { reason } => {
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

fn finalization_label(state: RepositoryRunCacheFinalState, finalize_ms: Option<u64>) -> String {
    let state = match state {
        RepositoryRunCacheFinalState::Pending => "pending",
        RepositoryRunCacheFinalState::Ready => "ready",
        RepositoryRunCacheFinalState::Evicted => "evicted",
    };
    finalize_ms.map_or_else(
        || state.to_string(),
        |milliseconds| format!("{state} · finalized {}", duration_label(milliseconds)),
    )
}

fn cold_reason_label(reason: RepositoryRunCacheColdReason) -> &'static str {
    match reason {
        RepositoryRunCacheColdReason::MetadataMissing => "no reusable entry for this identity",
        RepositoryRunCacheColdReason::MetadataInvalid => "cache metadata invalid",
        RepositoryRunCacheColdReason::MetadataNotReady => "cached volume not ready",
        RepositoryRunCacheColdReason::VolumeMissing => "cached volume missing",
        RepositoryRunCacheColdReason::VolumeInvalid => "cached volume invalid",
        RepositoryRunCacheColdReason::BackingDirectoryMissing => "cache backing directory missing",
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

fn short_oid(oid: &str) -> &str {
    oid.get(..7).unwrap_or(oid)
}

pub(super) fn run_state_label(state: impl Into<scope_domain::runs::run::RunState>) -> &'static str {
    use scope_domain::runs::run::RunState;
    match state.into() {
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
        scope_api_contract::RepositoryRunAttemptState::Dispatching => "dispatching",
        scope_api_contract::RepositoryRunAttemptState::Running => "running",
        scope_api_contract::RepositoryRunAttemptState::Succeeded => "succeeded",
        scope_api_contract::RepositoryRunAttemptState::Failed => "failed",
        scope_api_contract::RepositoryRunAttemptState::Canceled => "canceled",
        scope_api_contract::RepositoryRunAttemptState::Lost => "lost",
    }
}
