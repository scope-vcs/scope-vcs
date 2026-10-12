import assert from 'node:assert/strict'
import test from 'node:test'
import type { RequestCheckResponse, RequestChecksResponse } from '@/api/types.generated'
import { requestCheckGroups, requestChecksSummary } from './request-check-rows'

type GitHubCheck = Extract<RequestCheckResponse, { provider: 'github' }>

const github = (
  name: string,
  status: GitHubCheck['status'],
  conclusion: GitHubCheck['conclusion'] = null,
  run: GitHubCheck['run'] = null,
): RequestCheckResponse => ({
  conclusion,
  details_url: status ? 'https://github.com/o/r/runs/1' : null,
  name,
  provider: 'github',
  run,
  status,
})

const checks = (
  list: RequestCheckResponse[],
  push: RequestChecksResponse['github_push'] = null,
) => ({ state: 'started', checks: list, github_push: push }) as RequestChecksResponse

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
    provider: 'native',
    workflow: null,
    label: 'starting',
    leaf: 'checks',
    name: 'checks',
    parents: [],
    run: { id: 'run_a' },
    state: 'dispatching',
    tone: 'waiting',
  })
  assert.deepEqual(
    rows({ ...native, run_id: null, run_state: null })[0],
    {
      key: 'native:/.scope/runs/checks.yml',
      provider: 'native',
      workflow: null,
      label: 'not started',
      leaf: 'checks',
      name: 'checks',
      parents: [],
      run: null,
      state: 'pending',
      tone: 'waiting',
    },
  )
})

test('a GitHub check shows its conclusion, then its status, and opens its job on Scope', () => {
  const run = { run_id: '42', workflow_name: 'Build and test', job_id: '7' }
  assert.deepEqual(rows(github('ci / unit / test', 'completed', 'timed_out', run))[0], {
    key: 'github:ci / unit / test',
    provider: 'github',
    workflow: { id: '42', name: 'Build and test' },
    label: 'timed out',
    leaf: 'test',
    name: 'ci / unit / test',
    parents: ['ci', 'unit'],
    run: { id: '42', hash: 'run-job-7' },
    state: 'failed',
    tone: 'danger',
  })
  assert.equal(rows(github('ci', 'completed', 'neutral'))[0]!.state, 'succeeded')
  assert.equal(rows(github('ci', 'in_progress'))[0]!.label, 'in progress')
  assert.equal(rows(github('ci', null))[0]!.label, 'waiting')
  assert.equal(rows(github('ci', 'completed', 'success'))[0]!.run, null)
})

test('the summary leads with what is left and lists only checks that need someone', () => {
  const running = requestChecksSummary(checks([
    github('lint', 'completed', 'success'),
    github('ci / cli', null),
    github('ci / web', 'completed', 'skipped'),
    github('ci / api', 'in_progress'),
  ]))
  assert.deepEqual(running.lead, { state: 'running', text: 'CI running' })
  assert.equal(running.counts, '2 of 3 left · 1 passed · 1 skipped')
  assert.deepEqual(running.attention.map((row) => row.name), ['ci / api', 'ci / cli'])
  assert.equal(running.all.length, 4)

  const failed = requestChecksSummary(checks([
    github('ci / api', 'in_progress'),
    github('ci / web', 'completed', 'failure'),
    github('lint', 'completed', 'success'),
  ]))
  assert.deepEqual(failed.lead, { state: 'failed', text: 'CI failed' })
  assert.equal(failed.counts, '1 failed · 1 left · 1 passed')
  assert.deepEqual(failed.attention.map((row) => row.name), ['ci / web', 'ci / api'])

  const passed = requestChecksSummary(checks([
    github('lint', 'completed', 'success'),
    github('web', 'completed', 'skipped'),
  ]))
  assert.deepEqual(passed.lead, { state: 'succeeded', text: 'CI passed' })
  assert.equal(passed.counts, '1 passed · 1 skipped')
  assert.deepEqual(passed.attention, [])

  assert.equal(requestChecksSummary(checks([])).lead, null)

  const canceled = requestChecksSummary(checks([
    { provider: 'native', workflow_path: '/a.yml', workflow_name: 'a', run_id: 'run_a', run_state: 'succeeded' },
    { provider: 'native', workflow_path: '/b.yml', workflow_name: 'b', run_id: 'run_b', run_state: 'canceled' },
  ]))
  assert.deepEqual(canceled.lead, { state: 'failed', text: 'CI failed' })
  assert.deepEqual(canceled.attention.map((row) => [row.name, row.label]), [['b', 'canceled']])
})

test('the summary says when checks are starting or could not start, without naming the host', () => {
  const push = (state: 'sending' | 'failed', error: string | null) =>
    ({ state, branch: 'scope/requests/req_1', error })
  const waiting = [github('ci', null)]

  const sending = requestChecksSummary(checks(waiting, push('sending', null)))
  assert.deepEqual(sending.lead, { state: 'running', text: 'Starting CI' })
  assert.equal(sending.startError, null)
  assert.deepEqual(
    requestChecksSummary(checks(waiting, push('sending', 'remote rejected'))).startError,
    { text: 'The last attempt failed: remote rejected', failed: false },
  )

  const failed = requestChecksSummary(checks(waiting, push('failed', 'remote rejected')))
  assert.deepEqual(failed.lead, { state: 'failed', text: 'CI couldn’t start' })
  assert.deepEqual(failed.startError, { text: 'remote rejected', failed: true })
})

test('jobs group by their actual workflow run, independently of job paths or shared workflow names', () => {
  const run = { run_id: '42', workflow_name: 'Build and test', job_id: '7' }
  const groups = requestCheckGroups(rows(
    github('validate / server / api', 'in_progress', null, run),
    github('lint', 'completed', 'success', { ...run, job_id: '8' }),
    github('validate / cli', 'queued', null, { ...run, run_id: '43', job_id: '9' }),
    github('validate / missing', null),
    { provider: 'native', workflow_path: '/native.yml', workflow_name: 'Native', run_id: 'run_native', run_state: 'running' },
  ))
  const workflows = groups.filter((group) => group.kind === 'workflow')
  assert.deepEqual(workflows.map((group) => ({
    name: group.name,
    runId: group.runId,
    jobs: group.jobs.map((job) => job.name),
  })), [
    { name: 'Build and test', runId: '42', jobs: ['lint', 'validate / server / api'] },
    { name: 'Build and test', runId: '43', jobs: ['validate / cli'] },
  ])
  const unassigned = groups.find((group) => group.kind === 'unassigned')
  assert.deepEqual(unassigned?.jobs.map((job) => job.name), ['validate / missing'])
  const native = groups.find((group) => group.kind === 'native')
  assert.equal(native?.row.name, 'Native')
})
