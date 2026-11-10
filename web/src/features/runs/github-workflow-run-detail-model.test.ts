import assert from 'node:assert/strict'
import test from 'node:test'
import type {
  GitHubWorkflowJobResponse,
  GitHubWorkflowRunDetailResponse,
  GitHubWorkflowRunResponse,
} from '@/api/types.generated'
import {
  githubJobFailedStep,
  githubJobLog,
  githubJobLogAbsence,
  githubOpeningStep,
  githubRunCanChange,
  isGitHubRunId,
} from './github-workflow-run-detail-model'
import { runJobLinkForHash, runJobPanelId, runLogLineId } from './run-job-ids'

function job(id: number, overrides: Partial<GitHubWorkflowJobResponse> = {}): GitHubWorkflowJobResponse {
  return {
    id,
    name: `job ${id}`,
    status: 'completed',
    conclusion: 'success',
    started_at_unix: 100,
    completed_at_unix: 160,
    html_url: `https://github.com/octo/repo/actions/runs/7/job/${id}`,
    steps: [],
    ...overrides,
  }
}

function detail(
  run: Partial<GitHubWorkflowRunResponse>,
  jobs: GitHubWorkflowJobResponse[],
): GitHubWorkflowRunDetailResponse {
  return {
    run: {
      id: 7, workflow_name: 'ci', branch: 'main', head_oid: 'a'.repeat(40), event: 'push',
      status: 'completed', conclusion: 'success', html_url: 'https://github.com/octo/repo/actions/runs/7',
      run_started_at_unix: 100, updated_at_unix: 200, request_id: null, ...run,
    },
    jobs,
    jobs_not_read_yet: false,
    opening_job_id: jobs[0]?.id ?? null,
    opening_job_log: null,
  }
}

test('GitHub numbers its runs while Scope prefixes its own', () => {
  assert.equal(isGitHubRunId('12345678901'), true)
  for (const runId of ['run_01h', '', '12a', '-1']) assert.equal(isGitHubRunId(runId), false, runId)
})

test('a run-job hash names a job, and a line hash also names its step and line', () => {
  const keys = ['2', 'build linux']
  assert.deepEqual(runJobLinkForHash(keys, `#${runJobPanelId('build linux')}`), { key: 'build linux', step: null, line: null })
  assert.deepEqual(runJobLinkForHash(keys, `#${runLogLineId('2', 4, 435)}`), { key: '2', step: 4, line: 435 })
  for (const hash of ['#run-job-3', '#run-job-3:4:L1', '#run-job-2:4', '#ci', '']) {
    assert.equal(runJobLinkForHash(keys, hash), null, hash)
  }
})

test('a run can change until it and every job it lists complete', () => {
  assert.equal(githubRunCanChange(detail({}, [job(1)])), false)
  assert.equal(githubRunCanChange(detail({}, [])), true)
  assert.equal(githubRunCanChange(detail({ status: 'in_progress', conclusion: null }, [job(1)])), true)
  assert.equal(githubRunCanChange(detail({}, [job(1, { status: 'queued', conclusion: null })])), true)
})

test('a failed job names the step it stopped at', () => {
  const steps = [
    { number: 1, name: 'Set up job', status: 'completed', conclusion: 'success', started_at_unix: 100, completed_at_unix: 101 },
    { number: 2, name: 'Test', status: 'completed', conclusion: 'failure', started_at_unix: 101, completed_at_unix: 150 },
  ] satisfies GitHubWorkflowJobResponse['steps']
  assert.equal(githubJobFailedStep(job(1, { conclusion: 'failure', steps }))?.name, 'Test')
  assert.equal(githubJobFailedStep(job(1, { steps })), null)
  assert.equal(githubOpeningStep(job(1, { conclusion: 'failure', steps })), 2)
  assert.equal(githubOpeningStep(job(1, { steps })), 2, 'a passed job opens its longest step')
})

test('job log lines go to the last step started by their timestamp second', () => {
  const steps = [
    { number: 1, started_at_unix: Date.parse('2026-10-05T12:00:01Z') / 1000 },
    { number: 2, started_at_unix: Date.parse('2026-10-05T12:00:03Z') / 1000 },
    { number: 3, started_at_unix: null },
    { number: 4, started_at_unix: Date.parse('2026-10-05T12:00:05Z') / 1000 },
  ]
  const log = githubJobLog([
    '\uFEFF2026-10-05T12:00:00.5000000Z before the first step',
    '2026-10-05T12:00:02.9999999Z ##[group]Set up',
    'continued without a timestamp',
    '2026-10-05T12:00:03.0000000Z ##[group]Check out',
    '2026-10-05T12:00:04.1000000Z fetched',
    '2026-10-05T12:00:09.0000000Z ##[error]failed',
    '',
  ].join('\n'), steps)
  const byStep = [...log.stepLines].map(([step, lines]) => [step, lines.map((line) => [line.number, line.text, line.group])])
  assert.deepEqual(byStep, [
    [1, [[1, 'before the first step', null], [2, 'Set up', null], [3, 'continued without a timestamp', 2]]],
    [2, [[4, 'Check out', null], [5, 'fetched', 4]]],
    [3, []],
    [4, [[6, 'failed', null]]],
  ])
  assert.equal(log.lines[0]!.time, '2026-10-05T12:00:00.5000000Z')
  assert.equal(log.lines[2]!.time, null)
  assert.deepEqual(log.lines.map((line) => line.number), [1, 2, 3, 4, 5, 6])
})

test('a job without a log says why instead of offering a retry', () => {
  assert.equal(
    githubJobLogAbsence({ state: 'not_run' }, { conclusion: 'skipped' }),
    'This job was skipped, so it has no log.',
  )
  assert.equal(
    githubJobLogAbsence({ state: 'not_run' }, { conclusion: 'cancelled' }),
    'This job didn\'t run, so it has no log.',
  )
  assert.equal(
    githubJobLogAbsence({ state: 'expired' }, { conclusion: 'success' }),
    'This log is no longer available.',
  )
})
