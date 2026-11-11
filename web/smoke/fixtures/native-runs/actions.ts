import type { RunStepLogsInput } from '@/api/types'
import type { RepositoryRunDetailResponse, RepositoryRunSummaryResponse } from '@/api/types.generated'

const handoff = typeof window === 'undefined' ? null : window.__nativeRunHandoff
export const now = handoff?.now ?? Math.floor(Date.now() / 1000)
export const run: RepositoryRunSummaryResponse = {
  id: 'run-1', workflow_name: 'tests', git_oid: 'abcdef123456', trigger: 'manual', state: 'running',
  cancellation_requested: false, created_at_unix: now - 60, updated_at_unix: now,
  completed_at_unix: null, can_cancel: true, can_retry: false,
}
export const history = { runs: [run], next_cursor: 'older' }
const retainedLog = { logs: [{ position: 1, sequence: 1, text: 'retained build output', byte_length: 21, created_at_unix: now }], next_after: 1, has_earlier: false, has_more: false, logs_truncated: false }
export const detail: RepositoryRunDetailResponse = { run, jobs: [{
  job: { key: 'test', needs: [], pinned_container_image: 'image', state: 'running', created_at_unix: now, started_at_unix: now, updated_at_unix: now, completed_at_unix: null },
  attempts: [{ id: 'attempt', number: 1, external_run_id: null, runtime_version: '1', state: 'running', created_at_unix: now, started_at_unix: now,
    completed_at_unix: null, terminal_reason: null, cache_setup: null, caches: [], steps: [{ index: 0, name: 'Build', command: 'make build', state: 'running', started_at_unix: now, completed_at_unix: null, exit_code: null }] }],
  opening_step: { attempt_id: 'attempt', step_index: 0 },
}], opening: { job_key: 'test', step: { attempt_id: 'attempt', step_index: 0, log: retainedLog } } }
const longRun = typeof location !== 'undefined' && new URLSearchParams(location.search).has('long')
const chunk = (position: number) => {
  const text = Array.from({ length: 5 }, (_, line) => `chunk ${position} line ${line}\n`).join('')
  return { position, sequence: position, text, byte_length: text.length, created_at_unix: now }
}
const longLogs = Array.from({ length: 40 }, (_, index) => chunk(index + 1))
const firstAttemptLog = [{ position: 1, sequence: 1, text: 'first attempt ran out of memory\n', byte_length: 32, created_at_unix: now }]
const logPage = (logs: typeof longLogs, hasEarlier: boolean) => ({
  logs, next_after: logs.at(-1)?.position ?? 0, has_earlier: hasEarlier, has_more: false, logs_truncated: false,
})
const retriedJob = detail.jobs[0]!
const longDetail: RepositoryRunDetailResponse = {
  ...detail,
  jobs: [{
    ...retriedJob,
    attempts: [
      { ...retriedJob.attempts[0]!, id: 'attempt-1', state: 'failed', completed_at_unix: now,
        steps: retriedJob.attempts[0]!.steps.map((step) => ({ ...step, state: 'failed', completed_at_unix: now, exit_code: 137 })) },
      { ...retriedJob.attempts[0]!, number: 2 },
    ],
  }],
  opening: { job_key: 'test', step: { attempt_id: 'attempt', step_index: 0, log: logPage(longLogs.slice(20), true) } },
}
async function loadLongLogs(data: RunStepLogsInput) {
  if (data.attempt_id === 'attempt-1') return logPage(firstAttemptLog, false)
  const { after, before } = data
  if (before != null) return logPage(longLogs.filter((log) => log.position < before), false)
  return logPage(longLogs.filter((log) => log.position > (after ?? 20)), after == null)
}
export const workflows = { workflows: [], native_runs_available: true }
export const initialPage = { kind: 'native' as const, history, workflows, workflowsError: null }
export const seeded = typeof location === 'undefined' || !new URLSearchParams(location.search).has('client')
export const loads = { ...(handoff?.loads ?? { history: seeded ? 1 : 0, detail: seeded ? 1 : 0, workflows: seeded ? 1 : 0, logs: 0 }) }
let hold = false
let permitted = true
let nextDetail = longRun ? longDetail : detail
const releases: Array<() => void> = []
async function wait() { if (hold) await new Promise<void>((resolve) => releases.push(resolve)) }
if (typeof window !== 'undefined') Object.assign(window, {
  loads,
  holdLoads: () => { hold = true },
  releaseLoads: () => { hold = false; releases.splice(0).forEach((resolve) => resolve()) },
  setPermitted: (value: boolean) => { permitted = value },
  appendOutput: () => { longLogs.push(chunk(longLogs.length + 1)) },
  completeRun: () => { nextDetail = { ...detail, run: { ...run, state: 'succeeded', can_cancel: false, can_retry: true, completed_at_unix: now + 1, updated_at_unix: now + 1 } } },
})
export async function loadRepoRunPage() { loads.history++; loads.workflows++; await wait(); return permitted ? { ...initialPage, history: { ...history, runs: [nextDetail.run] } } : null }
export async function loadRepoRunHistory({ data }: { data: { after?: string } }) {
  loads.history++; await wait()
  return permitted ? data.after ? { runs: [{ ...nextDetail.run, id: 'older-run', workflow_name: 'older tests' }], next_cursor: null } : { ...history, runs: [nextDetail.run] } : null
}
export async function loadRepoRunWorkflows() { loads.workflows++; await wait(); return workflows }
export async function loadDetail() { loads.detail++; await wait(); if (!permitted) throw new Error('Run denied'); return nextDetail }
export async function loadLogs(input: RunStepLogsInput) {
  loads.logs++; await wait(); return longRun ? loadLongLogs(input) : retainedLog
}
export const loadRepoRunDetail = loadDetail
export async function cancelRun() { nextDetail = { ...detail, run: { ...run, state: 'cancelled', can_cancel: false, can_retry: true, completed_at_unix: now + 1, updated_at_unix: now + 1 } } }
export async function loadRepoGitHubWorkflowRuns() { return { configured: false, github: null } }
export async function loadRepoGitHubWorkflowNames() { return { workflows: [] } }
export async function loadRepoGitHubWorkflowRun(): Promise<never> { throw new Error('Unused fixture GitHub run') }
export async function loadRepoSettingsData() { throw new Error('Unused fixture settings') }
export async function startRepoGitHubAuthorization() { throw new Error('Unused fixture authorization') }
