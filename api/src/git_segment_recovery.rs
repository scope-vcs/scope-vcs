use crate::background_work::{BackgroundLoop, BackgroundWork, Cadence, Readiness};
use crate::{error::ApiError, persistence::unix_now, state::AppState};
use scope_domain::repository::git::GitSegmentUploadState;
use std::time::Duration;
use tokio::time::MissedTickBehavior;

const GIT_SEGMENT_STALE_SECONDS: u64 = 15 * 60;
const GIT_SEGMENT_RECOVERY_BATCH_SIZE: u64 = 100;
const GIT_SEGMENT_RECOVERY_INTERVAL: Duration = Duration::from_secs(10 * 60);

async fn recover_stale_git_segments(state: &AppState) -> Result<(), ApiError> {
    let started = std::time::Instant::now();
    let now = unix_now()?;
    let cutoff = now.saturating_sub(GIT_SEGMENT_STALE_SECONDS);
    let uploads = state
        .metadata
        .repositories()
        .load_stale_git_segment_uploads(cutoff, GIT_SEGMENT_RECOVERY_BATCH_SIZE)
        .await?;
    let candidates = uploads.len();
    let mut deleted = 0_u64;
    let mut skipped = 0_u64;
    let mut failed = 0_u64;
    for upload in uploads {
        let may_delete = match upload.state {
            GitSegmentUploadState::Uploading | GitSegmentUploadState::Ready => {
                state
                    .metadata
                    .repositories()
                    .abandon_git_segment_upload(&upload.segment_id, now)
                    .await?
            }
            GitSegmentUploadState::Deleting => true,
            GitSegmentUploadState::Published
            | GitSegmentUploadState::Retained
            | GitSegmentUploadState::Deleted => false,
        };
        if !may_delete {
            skipped += 1;
            continue;
        }
        if let Err(error) = state
            .git_segment_store
            .cleanup_remote_bounded(&upload.object_key)
            .await
        {
            failed += 1;
            tracing::warn!(
                repository_id = upload.repository_id,
                segment_id = upload.segment_id,
                error = %error,
                "stale Git segment remote cleanup failed"
            );
            continue;
        }
        let mut local_cleanup_failed = false;
        if let Err(error) = state
            .git_segment_store
            .cleanup_local(&upload.repository_id, &upload.segment_id)
            .await
        {
            local_cleanup_failed = true;
            failed += 1;
            tracing::warn!(
                repository_id = upload.repository_id,
                segment_id = upload.segment_id,
                error = %error,
                "stale Git segment local cleanup failed"
            );
        }
        state
            .metadata
            .repositories()
            .mark_git_segment_upload_deleted(&upload.segment_id, unix_now()?)
            .await?;
        if !local_cleanup_failed {
            deleted += 1;
        }
    }
    tracing::info!(
        success = failed == 0,
        duration_us = started.elapsed().as_micros(),
        candidates,
        deleted,
        skipped,
        failed,
        "stale Git segment recovery sweep completed"
    );
    Ok(())
}

impl BackgroundWork {
    pub(crate) fn start_git_segment_recovery(&mut self) {
        self.spawn(
            BackgroundLoop {
                name: "stale Git segment recovery",
                cadence: Cadence::Fixed {
                    period: GIT_SEGMENT_RECOVERY_INTERVAL,
                    missed_ticks: MissedTickBehavior::Burst,
                },
                readiness: Readiness::Ungated,
            },
            |state| async move { recover_stale_git_segments(&state).await },
        );
    }
}
