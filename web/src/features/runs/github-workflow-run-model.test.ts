import assert from 'node:assert/strict'
import test from 'node:test'
import type { GitHubWorkflowRunResponse } from '@/api/types.generated'
import { githubWorkflowRunRow } from './github-workflow-run-model'

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
