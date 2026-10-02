import type { GitHubWorkflowRunListResponse, GitHubWorkflowRunResponse } from '@/api/types.generated'
import { githubRunResult } from './github-run-status'

/** A Runs page's GitHub runs: every page loaded so far, merged. */
export type GitHubWorkflowRunPages = {
  list: GitHubWorkflowRunListResponse
  pages: number
}

/**
 * The list with `next` after it. A run that moved up a page while the list
 * was read appears once, where it was first listed.
 */
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

/**
 * Reads the first `pages` pages again from the top, so new runs push older
 * ones down without the list losing its depth.
 */
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

/** The workflows the filter offers, keeping the chosen one listed. */
export function githubWorkflowFilterOptions(workflows: string[], selected: string | null) {
  return selected === null || workflows.includes(selected)
    ? workflows
    : [...workflows, selected].sort()
}

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
