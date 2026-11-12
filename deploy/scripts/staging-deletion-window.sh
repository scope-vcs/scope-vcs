#!/usr/bin/env bash
source_blob_queue_delay_seconds=600
staging_deletion_window_seconds=$((8 * 24 * 60 * 60))
runner_clock_skew_allowance_seconds=300

defer_staging_deletions_sql() {
  local since="$1"
  [[ "$since" =~ ^[1-9][0-9]*$ ]] || { echo 'Deletion deferral needs a Unix start time.' >&2; return 2; }
  cat <<SQL
UPDATE public.scope_orphan_object_jobs SET next_run_at_unix = updated_at_unix + $staging_deletion_window_seconds
  WHERE updated_at_unix >= $since AND attempts = 0 AND completed_at_unix IS NULL
    AND next_run_at_unix - updated_at_unix = $source_blob_queue_delay_seconds;
UPDATE public.scope_repo_storage_cleanup_jobs SET next_run_at_unix = updated_at_unix + $staging_deletion_window_seconds
  WHERE updated_at_unix >= $since AND attempts = 0 AND completed_at_unix IS NULL
    AND next_run_at_unix = updated_at_unix;
UPDATE public.scope_request_media_cleanup_jobs SET available_at_unix = updated_at_unix + $staging_deletion_window_seconds
  WHERE updated_at_unix >= $since AND attempt = 0 AND completed_at_unix IS NULL
    AND state = 'Queued' AND lease_token IS NULL;
SQL
}
