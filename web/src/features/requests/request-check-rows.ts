import type {
  GitHubCheckConclusion,
  GitHubCheckStatus,
  RequestCheckResponse,
} from '@/api/types.generated'
import { runStatus } from '../runs/run-status'

/** One check as the request page lists it, with where its logs are. */
export type RequestCheckRow = {
  key: string
  name: string
  provider: 'Scope' | 'GitHub'
  /** A state in the runs vocabulary, so checks share the run status icons. */
  state: string
  label: string
  logs: { runId: string } | { href: string } | null
}

type CheckResult = { state: string; label: string }

const GITHUB_STATUS: Record<GitHubCheckStatus, CheckResult> = {
  queued: { state: 'queued', label: 'queued' },
  in_progress: { state: 'running', label: 'in progress' },
  completed: { state: 'pending', label: 'completed' },
  waiting: { state: 'queued', label: 'waiting' },
  requested: { state: 'queued', label: 'requested' },
  pending: { state: 'queued', label: 'pending' },
}

const GITHUB_CONCLUSION: Record<GitHubCheckConclusion, CheckResult> = {
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

export function requestCheckRow(check: RequestCheckResponse): RequestCheckRow {
  if (check.provider === 'native') {
    return {
      key: `native:${check.workflow_path}`,
      name: check.workflow_name,
      provider: 'Scope',
      // A recorded check without a run is waiting, not passing.
      state: check.run_state ?? 'pending',
      label: check.run_state ? runStatus(check.run_state).label : 'not started',
      logs: check.run_id ? { runId: check.run_id } : null,
    }
  }
  const result: CheckResult = check.conclusion
    ? GITHUB_CONCLUSION[check.conclusion]
    : check.status
      ? GITHUB_STATUS[check.status]
      : { state: 'pending', label: 'no run yet' }
  return {
    key: `github:${check.name}`,
    name: check.name,
    provider: 'GitHub',
    ...result,
    logs: check.details_url ? { href: check.details_url } : null,
  }
}
