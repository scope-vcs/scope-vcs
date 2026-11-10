import type {
  GitHubWorkflowJobLogResponse,
  GitHubWorkflowJobResponse,
  GitHubWorkflowRunDetailResponse,
  GitHubWorkflowStepResponse,
} from '@/api/types.generated'
import { githubRunResult } from './github-run-status'
import { readLogLines, type LogLine, type LogStamp } from './run-log-lines'

export function isGitHubRunId(runId: string) {
  return /^\d+$/.test(runId)
}

export function githubJobKey(job: Pick<GitHubWorkflowJobResponse, 'id'>) {
  return String(job.id)
}

export function githubRunCanChange({ jobs, run }: GitHubWorkflowRunDetailResponse) {
  return run.status !== 'completed' || jobs.length === 0 || jobs.some((job) => job.status !== 'completed')
}

export function githubJobFailedStep(job: GitHubWorkflowJobResponse) {
  return githubRunResult(job.status, job.conclusion).state === 'failed'
    ? job.steps.find((step) => githubRunResult(step.status, step.conclusion).state === 'failed') ?? null
    : null
}

export type GitHubJobLog = {
  lines: LogLine[]
  stepLines: ReadonlyMap<number, LogLine[]>
}

export function githubJobLog(
  text: string,
  steps: readonly Pick<GitHubWorkflowStepResponse, 'number' | 'started_at_unix'>[],
): GitHubJobLog {
  const lines = readLogLines(text, { stepsOf: (stamps) => githubLineSteps(stamps, steps) })
  const stepLines = new Map<number, LogLine[]>(steps.map((step) => [step.number, []]))
  for (const line of lines) stepLines.get(line.step)?.push(line)
  return { lines, stepLines }
}

function githubLineSteps(
  stamps: readonly LogStamp[],
  steps: readonly Pick<GitHubWorkflowStepResponse, 'number' | 'started_at_unix'>[],
) {
  const started = steps.filter((step) => step.started_at_unix !== null)
  const owners = new Array<number>(stamps.length).fill((started[0] ?? steps[0])?.number ?? 0)
  let previous = -1
  for (const [order, step] of started.entries()) {
    const second = step.started_at_unix!
    const stepsInSecond = started.slice(order).filter((later) => later.started_at_unix === second).length
    const groups = unclaimedGroups(stamps, second, previous)
    const boundary = groups.length >= stepsInSecond ? groups[0]! : firstLineFrom(stamps, second, previous)
    if (boundary === -1) break
    owners.fill(step.number, boundary)
    previous = boundary
  }
  return owners
}

function unclaimedGroups(stamps: readonly LogStamp[], second: number, previous: number) {
  const groups: number[] = []
  for (let index = previous + 1; index < stamps.length; index++) {
    const stamp = stamps[index]!
    if (stamp.second !== null && stamp.second > second) break
    if (stamp.group && stamp.second === second) groups.push(index)
  }
  return groups
}

function firstLineFrom(stamps: readonly LogStamp[], second: number, previous: number) {
  for (let index = Math.max(previous, 0); index < stamps.length; index++) {
    const stamp = stamps[index]!.second
    if (stamp !== null && stamp >= second) return index
  }
  return -1
}

export function githubOpeningStep(job: GitHubWorkflowJobResponse): number | null {
  const failed = githubJobFailedStep(job)
  if (failed) return failed.number
  const running = job.steps.find((step) => step.status === 'in_progress')
  if (running) return running.number
  let longest: GitHubWorkflowStepResponse | null = null
  for (const step of job.steps) {
    if (stepSeconds(step) >= 0 && (!longest || stepSeconds(step) > stepSeconds(longest))) longest = step
  }
  return longest?.number ?? null
}

function stepSeconds({ completed_at_unix, started_at_unix }: GitHubWorkflowStepResponse) {
  return started_at_unix === null || completed_at_unix === null ? -1 : completed_at_unix - started_at_unix
}

export function githubJobLogAbsence(
  log: Exclude<GitHubWorkflowJobLogResponse, { state: 'kept' | 'pending' }>,
  job: Pick<GitHubWorkflowJobResponse, 'conclusion'>,
) {
  if (log.state === 'expired') return 'This log is no longer available.'
  return job.conclusion === 'skipped'
    ? 'This job was skipped, so it has no log.'
    : 'This job didn\'t run, so it has no log.'
}
