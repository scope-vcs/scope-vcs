import assert from 'node:assert/strict'
import test from 'node:test'
import type {
  GitHubWorkflowJobResponse,
  GitHubWorkflowRunDetailResponse,
  GitHubWorkflowRunResponse,
} from '@/api/types.generated'
import {
  githubJobFailedStep,
  githubRunCanChange,
  isGitHubRunId,
  selectGitHubJob,
  stripLogTimestamps,
} from './github-workflow-run-detail-model'
import { jobKeyForHash, runJobPanelId } from './run-job-ids'

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
    jobs_unavailable: null,
  }
}

test('GitHub numbers its runs while Scope prefixes its own', () => {
  assert.equal(isGitHubRunId('12345678901'), true)
  for (const runId of ['run_01h', '', '12a', '-1']) assert.equal(isGitHubRunId(runId), false, runId)
})

test('a run opens on the job a link names, else its failure, else what is running', () => {
  const jobs = [
    job(1),
    job(2, { status: 'in_progress', conclusion: null }),
    job(3, { conclusion: 'timed_out' }),
    job(4, { conclusion: 'failure' }),
  ]
  const linked = jobKeyForHash(jobs.map((listed) => String(listed.id)), `#${runJobPanelId('2')}`)
  assert.equal(selectGitHubJob(jobs, linked)?.id, 2)
  assert.equal(selectGitHubJob(jobs, null)?.id, 3)
  assert.equal(selectGitHubJob(jobs, '99')?.id, 3)
  assert.equal(selectGitHubJob(jobs.slice(0, 2), null)?.id, 2)
  assert.equal(selectGitHubJob([job(1), job(5)], null)?.id, 1)
  assert.equal(selectGitHubJob([], null), null)
})

test('only a run-job hash names a job', () => {
  assert.equal(jobKeyForHash(['2'], '#run-job-3'), null)
  assert.equal(jobKeyForHash(['2'], '#ci'), null)
  assert.equal(jobKeyForHash(['2'], ''), null)
})

test('a run can change until it and every job complete', () => {
  assert.equal(githubRunCanChange(detail({}, [job(1)])), false)
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
})

test('log lines lose the timestamps GitHub stamps on them', () => {
  assert.equal(
    stripLogTimestamps(
      '﻿2026-10-05T12:00:00.1234567Z ##[group]Run tests\r\n' +
      '2026-10-05T12:00:01.0000000Z   ok 1 - adds\n' +
      '2026-10-05T12:00:02Z\n' +
      'printed 2026-10-05T12:00:03.0000000Z mid-line\n',
    ),
    '##[group]Run tests\r\n  ok 1 - adds\n\nprinted 2026-10-05T12:00:03.0000000Z mid-line\n',
  )
})
