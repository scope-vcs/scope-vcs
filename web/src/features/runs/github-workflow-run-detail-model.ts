import type {
  GitHubWorkflowJobResponse,
  GitHubWorkflowRunDetailResponse,
} from '@/api/types.generated'
import { githubRunResult } from './github-run-status'

export function isGitHubRunId(runId: string) {
  return /^\d+$/.test(runId)
}

export function githubJobKey(job: Pick<GitHubWorkflowJobResponse, 'id'>) {
  return String(job.id)
}

export function githubRunCanChange({ jobs, run }: GitHubWorkflowRunDetailResponse) {
  return run.status !== 'completed' || jobs.length === 0 || jobs.some((job) => job.status !== 'completed')
}

export function selectGitHubJob(
  jobs: readonly GitHubWorkflowJobResponse[],
  chosenKey: string | null,
): GitHubWorkflowJobResponse | null {
  return (chosenKey === null ? undefined : jobs.find((job) => githubJobKey(job) === chosenKey))
    ?? jobs.find((job) => githubRunResult(job.status, job.conclusion).state === 'failed')
    ?? jobs.find((job) => job.status === 'in_progress')
    ?? jobs[0]
    ?? null
}

export function githubJobFailedStep(job: GitHubWorkflowJobResponse) {
  return githubRunResult(job.status, job.conclusion).state === 'failed'
    ? job.steps.find((step) => githubRunResult(step.status, step.conclusion).state === 'failed') ?? null
    : null
}

const LOG_LINE_TIMESTAMP = /^﻿?\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z ?/gm

export function stripLogTimestamps(text: string) {
  return text.replace(LOG_LINE_TIMESTAMP, '')
}
