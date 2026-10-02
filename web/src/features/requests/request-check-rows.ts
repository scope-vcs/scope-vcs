import type { RequestCheckResponse } from '@/api/types.generated'
import { githubRunResult } from '../runs/github-run-status'
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
  const result = check.status
    ? githubRunResult(check.status, check.conclusion)
    : { state: 'pending', label: 'no run yet' }
  return {
    key: `github:${check.name}`,
    name: check.name,
    provider: 'GitHub',
    ...result,
    logs: check.details_url ? { href: check.details_url } : null,
  }
}
