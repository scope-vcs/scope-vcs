import assert from 'node:assert/strict'
import test from 'node:test'
import type { RequestCheckResponse, RequestChecksResponse } from '@/api/types.generated'
import { requestCheckTree, requestChecksSummary } from './request-check-rows'

type GitHubCheck = Extract<RequestCheckResponse, { provider: 'github' }>

const github = (
  name: string,
  status: GitHubCheck['status'],
  conclusion: GitHubCheck['conclusion'] = null,
): RequestCheckResponse => ({
  conclusion,
  details_url: status ? 'https://github.com/o/r/runs/1' : null,
  name,
  provider: 'github',
  status,
})

const checks = (
  list: RequestCheckResponse[],
  push: RequestChecksResponse['github_push'] = null,
) => ({ checks: list, github_push: push }) as RequestChecksResponse

const rows = (...list: RequestCheckResponse[]) => requestChecksSummary(checks(list)).all

test('a native check links its run, and one without a run has not started', () => {
  const native = {
    provider: 'native',
    run_id: 'run_a',
    run_state: 'dispatching',
    workflow_name: 'checks',
    workflow_path: '/.scope/runs/checks.yml',
  } as const
  assert.deepEqual(rows(native)[0], {
    key: 'native:/.scope/runs/checks.yml',
    label: 'starting',
    leaf: 'checks',
    name: 'checks',
    parents: [],
    runId: 'run_a',
    state: 'dispatching',
    tone: 'waiting',
  })
  assert.deepEqual(
    rows({ ...native, run_id: null, run_state: null })[0],
    {
      key: 'native:/.scope/runs/checks.yml',
      label: 'not started',
      leaf: 'checks',
      name: 'checks',
      parents: [],
      runId: null,
      state: 'pending',
      tone: 'waiting',
    },
  )
})

test('a GitHub check shows its conclusion, then its status, and never links GitHub', () => {
  assert.deepEqual(rows(github('ci / unit / test', 'completed', 'timed_out'))[0], {
    key: 'github:ci / unit / test',
    label: 'timed out',
    leaf: 'test',
    name: 'ci / unit / test',
    parents: ['ci', 'unit'],
    runId: null,
    state: 'failed',
    tone: 'danger',
  })
  assert.equal(rows(github('ci', 'completed', 'neutral'))[0]!.state, 'succeeded')
  assert.equal(rows(github('ci', 'in_progress'))[0]!.label, 'in progress')
  assert.equal(rows(github('ci', null))[0]!.label, 'waiting')
})

test('the summary leads with what is left and lists only checks that need someone', () => {
  const running = requestChecksSummary(checks([
    github('lint', 'completed', 'success'),
    github('ci / cli', null),
    github('ci / web', 'completed', 'skipped'),
    github('ci / api', 'in_progress'),
  ]))
  assert.deepEqual(running.lead, { state: 'running', text: '2 of 3 left' })
  assert.equal(running.counts, '1 passed · 1 skipped')
  assert.deepEqual(running.attention.map((row) => row.name), ['ci / api', 'ci / cli'])
  assert.equal(running.all.length, 4)

  const failed = requestChecksSummary(checks([
    github('ci / api', 'in_progress'),
    github('ci / web', 'completed', 'failure'),
    github('lint', 'completed', 'success'),
  ]))
  assert.deepEqual(failed.lead, { state: 'failed', text: '1 failed' })
  assert.equal(failed.counts, '1 left · 1 passed')
  assert.deepEqual(failed.attention.map((row) => row.name), ['ci / web', 'ci / api'])

  const passed = requestChecksSummary(checks([
    github('lint', 'completed', 'success'),
    github('web', 'completed', 'skipped'),
  ]))
  assert.deepEqual(passed.lead, { state: 'succeeded', text: 'All 1 passed' })
  assert.equal(passed.counts, '1 skipped')
  assert.deepEqual(passed.attention, [])

  assert.equal(requestChecksSummary(checks([])).lead, null)

  // A canceled native run blocks merging, so it is a failure, not a skip.
  const canceled = requestChecksSummary(checks([
    { provider: 'native', workflow_path: '/a.yml', workflow_name: 'a', run_id: 'run_a', run_state: 'succeeded' },
    { provider: 'native', workflow_path: '/b.yml', workflow_name: 'b', run_id: 'run_b', run_state: 'canceled' },
  ]))
  assert.deepEqual(canceled.lead, { state: 'failed', text: '1 failed' })
  assert.deepEqual(canceled.attention.map((row) => [row.name, row.label]), [['b', 'canceled']])
})

test('the summary says when checks are starting or could not start, without naming the host', () => {
  const push = (state: 'sending' | 'failed', error: string | null) =>
    ({ state, branch: 'scope/requests/req_1', error })
  const waiting = [github('ci', null)]

  const sending = requestChecksSummary(checks(waiting, push('sending', null)))
  assert.deepEqual(sending.lead, { state: 'running', text: 'Starting checks' })
  assert.equal(sending.startError, null)
  assert.deepEqual(
    requestChecksSummary(checks(waiting, push('sending', 'remote rejected'))).startError,
    { text: 'The last attempt failed: remote rejected', failed: false },
  )

  const failed = requestChecksSummary(checks(waiting, push('failed', 'remote rejected')))
  assert.deepEqual(failed.lead, { state: 'failed', text: 'Checks couldn’t start' })
  assert.deepEqual(failed.startError, { text: 'remote rejected', failed: true })
})

test('the full list nests checks under each workflow heading once', () => {
  const { all } = requestChecksSummary(checks([
    github('validate / server / web', 'completed', 'skipped'),
    github('lint', 'completed', 'success'),
    github('validate / cli', null),
    github('validate / server / api', 'completed', 'success'),
  ]))
  assert.deepEqual(
    requestCheckTree(all).map((line) =>
      line.kind === 'group' ? `${line.depth} # ${line.name}` : `${line.depth} ${line.row.leaf}`),
    ['0 lint', '0 # validate', '1 cli', '1 # server', '2 api', '2 web'],
  )
  // Workflows differing only in case keep their checks together.
  const cased = requestChecksSummary(checks([
    github('A / a', null),
    github('a / b', null),
    github('A / c', null),
  ])).all
  assert.deepEqual(
    requestCheckTree(cased).map((line) => line.kind === 'group' ? `# ${line.name}` : line.row.leaf),
    ['# a', 'b', '# A', 'a', 'c'],
  )
})
