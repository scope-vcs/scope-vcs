import assert from 'node:assert/strict'
import test from 'node:test'
import type { GitHubWorkflowRunDetailResponse, GitHubWorkflowRunResponse } from '@/api/types.generated'
import {
  githubWorkflowRunDetailIdentity,
  githubWorkflowRunDetailResource,
  seedGitHubWorkflowRunDetail,
} from './github-workflow-run-detail-resource'

test('a clicked run shows its header while its detail response is held', async () => {
  githubWorkflowRunDetailResource.clear()
  const run: GitHubWorkflowRunResponse = {
    id: 21, workflow_name: 'CI checks', branch: 'main', head_oid: 'a'.repeat(40),
    event: 'push', status: 'in_progress', conclusion: null,
    html_url: 'https://github.com/owner/repo/actions/runs/21',
    run_started_at_unix: 100, updated_at_unix: 120, request_id: null,
  }
  const identity = githubWorkflowRunDetailIdentity('viewer/repo/member', '21')
  seedGitHubWorkflowRunDetail('viewer/repo/member', run)
  let resolve!: (value: GitHubWorkflowRunDetailResponse) => void
  const response = new Promise<GitHubWorkflowRunDetailResponse>((done) => { resolve = done })
  const loading = githubWorkflowRunDetailResource.ensure(identity, '', () => response)
  assert.equal(githubWorkflowRunDetailResource.getSnapshot(identity).pending, true)
  assert.equal(githubWorkflowRunDetailResource.peek(identity)?.run.workflow_name, 'CI checks')
  assert.equal(githubWorkflowRunDetailResource.peek(identity)?.jobs_not_read_yet, true)
  resolve({ run: { ...run, status: 'completed', conclusion: 'success' }, jobs: [], jobs_not_read_yet: false })
  await loading
  assert.equal(githubWorkflowRunDetailResource.peek(identity)?.run.status, 'completed')
  githubWorkflowRunDetailResource.clear()
})
