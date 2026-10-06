import type { GitHubWorkflowRunListResponse, GitHubWorkflowRunResponse } from '@/api/types.generated'
import { githubRunResult } from './github-run-status'

export type GitHubWorkflowRunPages = {
  list: GitHubWorkflowRunListResponse
  pages: number
}

export function mergeNextPage(
  list: GitHubWorkflowRunListResponse,
  next: GitHubWorkflowRunListResponse,
): GitHubWorkflowRunListResponse {
  const seen = new Set(list.workflow_runs.map((run) => run.id))
  return {
    ...next,
    workflow_runs: [...list.workflow_runs, ...next.workflow_runs.filter((run) => !seen.has(run.id))],
  }
}

export async function reloadGitHubWorkflowRunPages(
  pages: number,
  loadPage: (after?: string) => Promise<GitHubWorkflowRunListResponse>,
): Promise<GitHubWorkflowRunPages> {
  let list = await loadPage()
  let loaded = 1
  const requested = new Set<string>()
  while (loaded < pages && list.next_cursor) {
    if (requested.has(list.next_cursor)) throw new Error('GitHub runs returned a repeated cursor.')
    requested.add(list.next_cursor)
    list = mergeNextPage(list, await loadPage(list.next_cursor))
    loaded += 1
  }
  return { list, pages: loaded }
}

export function githubWorkflowFilterOptions(workflows: string[], selected: string | null) {
  return selected === null || workflows.includes(selected)
    ? workflows
    : [...workflows, selected].sort()
}

export type GitHubWorkflowRunRow = {
  key: string
  name: string
  state: string
  label: string
  branch: string | null
  event: string
  commit: string
  requestId: string | null
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
    requestId: run.request_id,
    at: run.run_started_at_unix ?? run.updated_at_unix,
  }
}
