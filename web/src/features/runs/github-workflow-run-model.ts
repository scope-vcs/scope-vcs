import type { GitHubWorkflowRunResponse } from '@/api/types.generated'
import { githubRunResult } from './github-run-status'

/** One GitHub workflow run as the Runs page lists it. */
export type GitHubWorkflowRunRow = {
  key: string
  name: string
  /** A state in the runs vocabulary, for the shared status icon. */
  state: string
  label: string
  branch: string | null
  /** What started the run, as GitHub names it, such as `push`. */
  event: string
  commit: string
  href: string
  requestId: string | null
  /** When the run started, or when GitHub last changed it before it started. */
  at: number
}

export function githubWorkflowRunRow(run: GitHubWorkflowRunResponse): GitHubWorkflowRunRow {
  const result = githubRunResult(run.status, run.conclusion)
  return {
    key: String(run.id),
    name: run.workflow_name,
    state: result.state,
    label: result.label,
    branch: run.branch,
    event: run.event,
    commit: run.head_oid.slice(0, 7),
    href: run.html_url,
    requestId: run.request_id,
    at: run.run_started_at_unix ?? run.updated_at_unix,
  }
}
