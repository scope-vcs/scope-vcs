import assert from 'node:assert/strict'
import test from 'node:test'
import type { GitHubWorkflowRunListResponse, GitHubWorkflowRunResponse } from '@/api/types.generated'
import {
  githubWorkflowFilterOptions,
  githubWorkflowRunRow,
  mergeNextPage,
  reloadGitHubWorkflowRunPages,
} from './github-workflow-run-model'

function run(overrides: Partial<GitHubWorkflowRunResponse> = {}): GitHubWorkflowRunResponse {
  return {
    id: 21,
    workflow_name: 'ci',
    branch: 'scope/requests/req_1',
    head_oid: 'a'.repeat(40),
    event: 'push',
    status: 'in_progress',
    conclusion: null,
    html_url: 'https://github.com/octo/repo/actions/runs/21',
    run_started_at_unix: 100,
    updated_at_unix: 120,
    request_id: 'req_1',
    ...overrides,
  }
}

test('a running run on a request branch links GitHub and its request', () => {
  assert.deepEqual(githubWorkflowRunRow(run()), {
    key: '21',
    name: 'ci',
    state: 'running',
    label: 'in progress',
    branch: 'scope/requests/req_1',
    event: 'push',
    commit: 'aaaaaaa',
    href: 'https://github.com/octo/repo/actions/runs/21',
    requestId: 'req_1',
    at: 100,
  })
})

test('a finished run is described by its conclusion', () => {
  const row = githubWorkflowRunRow(run({ status: 'completed', conclusion: 'timed_out' }))
  assert.deepEqual([row.state, row.label], ['failed', 'timed out'])
})

test('a run without a start time or branch falls back to what is known', () => {
  const row = githubWorkflowRunRow(run({ branch: null, run_started_at_unix: null, request_id: null }))
  assert.equal(row.branch, null)
  assert.equal(row.at, 120)
  assert.equal(row.requestId, null)
})

function page(ids: number[], next_cursor: string | null): GitHubWorkflowRunListResponse {
  return {
    actions_url: 'https://github.com/octo/repo/actions',
    workflow_runs: ids.map((id) => run({ id })),
    workflows: ['ci', 'lint'],
    next_cursor,
  }
}

const ids = (list: GitHubWorkflowRunListResponse) => list.workflow_runs.map((listed) => listed.id)

test('the next page follows the list and a run that moved up meanwhile is listed once', () => {
  const merged = mergeNextPage(page([5, 4], 'b'), page([4, 3], null))
  assert.deepEqual(ids(merged), [5, 4, 3])
  assert.equal(merged.next_cursor, null)
})

test('a refresh reads as many pages again as were loaded, from the top', async () => {
  const pages: Record<string, GitHubWorkflowRunListResponse> = {
    first: page([6, 5], 'a'),
    a: page([4, 3], 'b'),
    b: page([2, 1], null),
  }
  const requested: (string | undefined)[] = []
  const loadPage = async (after?: string) => {
    requested.push(after)
    return pages[after ?? 'first']
  }
  const reloaded = await reloadGitHubWorkflowRunPages(2, loadPage)
  assert.deepEqual(ids(reloaded.list), [6, 5, 4, 3])
  assert.equal(reloaded.list.next_cursor, 'b')
  assert.equal(reloaded.pages, 2)
  assert.deepEqual(requested, [undefined, 'a'])
  // A shorter list than before keeps only the pages it has.
  const shorter = await reloadGitHubWorkflowRunPages(5, async (after) => (after ? page([1], null) : page([2], 'x')))
  assert.deepEqual([ids(shorter.list), shorter.pages], [[2, 1], 2])
  await assert.rejects(reloadGitHubWorkflowRunPages(3, async () => page([1], 'loop')), /repeated cursor/)
})

test('the workflow filter keeps the chosen workflow listed', () => {
  assert.deepEqual(githubWorkflowFilterOptions(['ci', 'lint'], null), ['ci', 'lint'])
  assert.deepEqual(githubWorkflowFilterOptions(['ci', 'lint'], 'lint'), ['ci', 'lint'])
  assert.deepEqual(githubWorkflowFilterOptions(['lint'], 'deploy'), ['deploy', 'lint'])
})
