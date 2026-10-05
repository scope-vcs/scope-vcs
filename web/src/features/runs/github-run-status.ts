import type { GitHubCheckConclusion, GitHubCheckStatus } from '@/api/types.generated'

export type GitHubRunResult = { state: string; label: string }

const GITHUB_STATUS: Record<GitHubCheckStatus, GitHubRunResult> = {
  queued: { state: 'queued', label: 'queued' },
  in_progress: { state: 'running', label: 'in progress' },
  completed: { state: 'pending', label: 'completed' },
  waiting: { state: 'queued', label: 'waiting' },
  requested: { state: 'queued', label: 'requested' },
  pending: { state: 'queued', label: 'pending' },
}

const GITHUB_CONCLUSION: Record<GitHubCheckConclusion, GitHubRunResult> = {
  success: { state: 'succeeded', label: 'succeeded' },
  neutral: { state: 'succeeded', label: 'neutral' },
  skipped: { state: 'skipped', label: 'skipped' },
  failure: { state: 'failed', label: 'failed' },
  cancelled: { state: 'failed', label: 'cancelled' },
  timed_out: { state: 'failed', label: 'timed out' },
  action_required: { state: 'failed', label: 'action required' },
  stale: { state: 'failed', label: 'stale' },
  startup_failure: { state: 'failed', label: 'startup failure' },
}

export function githubRunResult(
  status: GitHubCheckStatus,
  conclusion: GitHubCheckConclusion | null,
): GitHubRunResult {
  return conclusion ? GITHUB_CONCLUSION[conclusion] : GITHUB_STATUS[status]
}
