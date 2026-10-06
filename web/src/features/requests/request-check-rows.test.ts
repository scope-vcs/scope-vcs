import assert from 'node:assert/strict'
import test from 'node:test'
import type { RequestCheckResponse } from '@/api/types.generated'
import { requestCheckRow } from './request-check-rows'

test('a native check links its run, and one without a run has not started', () => {
  const native = {
    provider: 'native',
    run_id: 'run_a',
    run_state: 'dispatching',
    workflow_name: 'checks',
    workflow_path: '/.scope/runs/checks.yml',
  } as const
  assert.deepEqual(requestCheckRow(native), {
    key: 'native:/.scope/runs/checks.yml',
    label: 'starting',
    logs: { runId: 'run_a' },
    name: 'checks',
    provider: 'Scope',
    state: 'dispatching',
  })
  assert.deepEqual(
    requestCheckRow({ ...native, run_id: null, run_state: null }),
    {
      key: 'native:/.scope/runs/checks.yml',
      label: 'not started',
      logs: null,
      name: 'checks',
      provider: 'Scope',
      state: 'pending',
    },
  )
})

test('a GitHub check shows its conclusion, then its status, and links GitHub', () => {
  const github = (
    status: Extract<RequestCheckResponse, { provider: 'github' }>['status'],
    conclusion: Extract<RequestCheckResponse, { provider: 'github' }>['conclusion'],
  ): RequestCheckResponse => ({
    conclusion,
    details_url: status ? 'https://github.com/o/r/runs/1' : null,
    name: 'ci / test',
    provider: 'github',
    run: null,
    status,
  })
  assert.deepEqual(requestCheckRow(github('completed', 'timed_out')), {
    key: 'github:ci / test',
    label: 'timed out',
    logs: { href: 'https://github.com/o/r/runs/1' },
    name: 'ci / test',
    provider: 'GitHub',
    state: 'failed',
  })
  assert.equal(requestCheckRow(github('completed', 'neutral')).state, 'succeeded')
  assert.equal(requestCheckRow(github('in_progress', null)).label, 'in progress')
  assert.deepEqual(requestCheckRow(github(null, null)), {
    key: 'github:ci / test',
    label: 'no run yet',
    logs: null,
    name: 'ci / test',
    provider: 'GitHub',
    state: 'pending',
  })
})
