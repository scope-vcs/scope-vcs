import type { RepositoryRunDetailResponse, RepositoryRunSummaryResponse } from '@/api/types.generated'

const handoff = typeof window === 'undefined' ? null : window.__nativeRunHandoff
export const now = handoff?.now ?? Math.floor(Date.now() / 1000)
export const run: RepositoryRunSummaryResponse = {
  id: 'run-1', workflow_name: 'tests', git_oid: 'abcdef123456', trigger: 'manual', state: 'running',
  cancellation_requested: false, created_at_unix: now - 60, updated_at_unix: now,
  completed_at_unix: null, can_cancel: true, can_retry: false,
}
export const history = { runs: [run], next_cursor: 'older' }
export const detail: RepositoryRunDetailResponse = { run, jobs: [{
  job: { key: 'test', needs: [], pinned_container_image: 'image', state: 'running', created_at_unix: now, started_at_unix: now, updated_at_unix: now, completed_at_unix: null },
  attempts: [{ id: 'attempt', number: 1, external_run_id: null, runtime_version: '1', state: 'running', created_at_unix: now, started_at_unix: now,
    completed_at_unix: null, terminal_reason: null, cache_setup: null, caches: [], steps: [{ index: 0, name: 'Build', command: 'make build', state: 'running', started_at_unix: now, completed_at_unix: null, exit_code: null }] }],
}] }
export const workflows = { workflows: [], native_runs_available: true }
export const initialPage = { kind: 'native' as const, githubConfigured: false, history, workflows, workflowsError: null }
export const seeded = typeof location === 'undefined' || !new URLSearchParams(location.search).has('client')
export const loads = { ...(handoff?.loads ?? { history: seeded ? 1 : 0, detail: seeded ? 1 : 0, workflows: seeded ? 1 : 0, logs: 0 }) }
let hold = false
let permitted = true
let nextDetail = detail
const releases: Array<() => void> = []
async function wait() { if (hold) await new Promise<void>((resolve) => releases.push(resolve)) }
if (typeof window !== 'undefined') Object.assign(window, {
  loads,
  holdLoads: () => { hold = true },
  releaseLoads: () => { hold = false; releases.splice(0).forEach((resolve) => resolve()) },
  setPermitted: (value: boolean) => { permitted = value },
  completeRun: () => { nextDetail = { ...detail, run: { ...run, state: 'succeeded', can_cancel: false, can_retry: true, completed_at_unix: now + 1, updated_at_unix: now + 1 } } },
})
export async function loadRepoRunPage() { loads.history++; loads.workflows++; await wait(); return permitted ? { ...initialPage, history: { ...history, runs: [nextDetail.run] } } : null }
export async function loadRepoRunHistory({ data }: { data: { after?: string } }) {
  loads.history++; await wait()
  return permitted ? data.after ? { runs: [{ ...nextDetail.run, id: 'older-run', workflow_name: 'older tests' }], next_cursor: null } : { ...history, runs: [nextDetail.run] } : null
}
export async function loadRepoRunWorkflows() { loads.workflows++; await wait(); return workflows }
export async function loadDetail() { loads.detail++; await wait(); if (!permitted) throw new Error('Run denied'); return nextDetail }
export async function loadLogs() { loads.logs++; await wait(); return { logs: [{ position: 1, sequence: 1, text: 'retained build output', byte_length: 21, created_at_unix: now }], next_after: 1, has_earlier: false, has_more: false, logs_truncated: false } }
export async function cancelRun() { nextDetail = { ...detail, run: { ...run, state: 'cancelled', can_cancel: false, can_retry: true, completed_at_unix: now + 1, updated_at_unix: now + 1 } } }
export async function loadRepoGitHubWorkflowRuns() { return { configured: false, github: null } }
export async function loadRepoSettingsData() { throw new Error('Unused fixture settings') }
export async function startRepoGitHubAuthorization() { throw new Error('Unused fixture authorization') }
