import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { readFileSync, readdirSync } from 'node:fs';
import { pathToFileURL } from 'node:url';

const directory = 'crates/scope-postgres/src/migrations';
const lockPath = `${directory}/sources.lock.json`;
export const migrationDigest = (contents) => createHash('sha256').update(contents).digest('hex');
const isDefinition = (name) => /^m\d+_[a-z0-9_]+\.(rs|sql|txt)$/.test(name);
export const retiredLockEntries = new Set([
  'baseline_ledger.txt', 'current_schema.sql', 'm0043_retire_git_manifests.rs',
  'm0044_request_attention.rs', 'm0045_dependency_analysis.rs',
  'm0046_drop_attempt_token_expiry.rs', 'm0047_drop_git_manifest_orphan_jobs.rs',
  'm0048_run_attempt_active_state_indexes.rs', 'm0049_run_execution_invariants.rs',
  'm0050_cache_upload_cleanup_leases.rs', 'm0051_request_media_budget_release.rs',
  'm0052_retire_apply_changes_permission.rs', 'm0053_request_ref_cleanup.rs',
  'm0054_request_attention_revision.rs', 'm0055_request_checks.rs', 'm0056_request_auto_merge.rs',
  'm0057_repository_invite_links.rs', 'm0058_repository_invite_emails.rs',
  'm0059_worker_history_permissions.rs', 'm0060_capacity_retries.rs',
  'm0061_public_request_check_source.rs', 'm0062_request_run_source_base.rs',
  'm0063_account_deletion.rs', 'm0064_repository_content_version.rs',
  'm0065_request_revision_rewrites.rs', 'm0066_native_runs_accounts.rs',
  'm0067_provider_neutral_request_checks.rs', 'm0068_github_connections.rs',
  'm0069_github_request_checks.rs', 'm0070_github_setup_checks_and_workflow_runs.rs',
  'm0071_public_check_commits.rs', 'm0072_github_run_imports.rs', 'm0073_github_workflow_jobs.rs',
  'm0074_history_path_indexes.rs', 'm0075_incremental_projection_read_models.rs',
  'm0076_view_ids.rs', 'm0077_custom_views.rs', 'm0078_repo_config_version.rs',
  'm0079_request_views.rs', 'm0080_github_workflow_job_reads.rs', 'm0081_github_job_log_reads.rs',
  'm0082_github_workflow_run_list.rs', 'm0083_request_workflow_changes.rs'
]);

export function checkMigrationSources(sources, locked, previous = {}) {
  const errors = [];
  for (const [name, digest] of Object.entries(previous)) {
    if (!retiredLockEntries.has(name) && locked[name] !== digest) errors.push(`${name}: an existing migration lock cannot change or disappear; add a new migration`);
  }
  for (const [name, digest] of Object.entries(locked)) {
    if (retiredLockEntries.has(name)) errors.push(`${name}: retired migration lock entry must not reappear`);
    else if (!isDefinition(name) || !/^[a-f0-9]{64}$/.test(digest)) errors.push(`${name}: invalid migration lock entry`);
    else if (sources[name] === undefined) errors.push(`${name}: locked migration source is missing`);
    else if (migrationDigest(sources[name]) !== digest) errors.push(`${name}: migration definition changed; add a new migration`);
  }
  for (const name of Object.keys(sources)) {
    if (!(name in locked)) errors.push(`${name}: new migration needs a SHA-256 entry in ${lockPath}`);
  }
  return errors;
}

function previousLock() {
  const event = process.env.GITHUB_EVENT_PATH
    ? JSON.parse(readFileSync(process.env.GITHUB_EVENT_PATH, 'utf8')) : {};
  const base = event.pull_request?.base?.sha;
  if (!base) return {};
  if (!/^[a-f0-9]{40}$/.test(base)) throw new Error('Invalid pull request base SHA');
  const entry = execFileSync('git', ['ls-tree', base, '--', lockPath], { encoding: 'utf8' });
  const isInitialLockAdoption = !entry.trim();
  if (isInitialLockAdoption) return {};
  return JSON.parse(execFileSync('git', ['show', `${base}:${lockPath}`], { encoding: 'utf8' }));
}

function main() {
  const sources = Object.fromEntries(readdirSync(directory).filter(isDefinition)
    .map((name) => [name, readFileSync(`${directory}/${name}`)]));
  const errors = checkMigrationSources(sources, JSON.parse(readFileSync(lockPath, 'utf8')), previousLock());
  if (errors.length) throw new Error(`Migration sources are immutable:\n${errors.join('\n')}`);
  process.stdout.write(`Verified ${Object.keys(sources).length} immutable migration definitions.\n`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) main();
