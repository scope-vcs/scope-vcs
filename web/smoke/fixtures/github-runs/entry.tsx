import { useState } from 'react'
import { createRoot } from 'react-dom/client'
import { createRootRoute, createRoute, createRouter, Link, Outlet, RouterProvider, useParams } from '@tanstack/react-router'
import { RepoShell } from '@/components/repo-shell'
import { RepoLayoutProvider } from '@/features/repo-detail/repo-layout-context'
import { invalidateRepoResources } from '@/features/repo-detail/repo-resource-invalidation'
import { repoResourceScope } from '@/features/repo-detail/repo-resource-scope'
import { GitHubWorkflowRunsPage } from '@/features/runs/github-workflow-runs'
import { GitHubWorkflowRunDetailPage } from '@/features/runs/github-workflow-run-detail'
import { RunsCiEmptyState } from '@/features/runs/runs-ci-empty-state'
import type { RepoGitHubWorkflowRunsInput, RepoParams } from '@/api/types'
import type { RepoLiveState } from '@/api/types'
import type {
  GitHubWorkflowJobLogResponse,
  GitHubWorkflowJobResponse,
  GitHubWorkflowRunDetailResponse,
  GitHubWorkflowRunListResponse,
  GitHubWorkflowRunResponse,
  GitHubWorkflowStepResponse,
} from '@/api/types.generated'
import '../shared/styles.css'

const now = Math.floor(Date.now() / 1000)
function run(id: number, overrides: Partial<GitHubWorkflowRunResponse>): GitHubWorkflowRunResponse {
  return {
    id, workflow_name: 'ci', branch: 'main', head_oid: `${id}`.padStart(40, 'a'), event: 'push',
    status: 'completed', conclusion: 'success', html_url: `https://github.com/octo/demo/actions/runs/${id}`,
    run_started_at_unix: now - id * 60, updated_at_unix: now - id * 60, request_id: null, ...overrides,
  }
}
const initialRuns: GitHubWorkflowRunListResponse = {
  actions_url: 'https://github.com/octo/demo/actions',
  workflow_runs: [
    run(1, { branch: 'scope/requests/req_1', status: 'in_progress', conclusion: null, request_id: 'req_1' }),
    run(2, { workflow_name: 'Release images and publish the container manifests for every platform', conclusion: 'failure' }),
    run(3, { branch: 'scope/setup-check' }),
  ],
  next_cursor: 'page-2',
}
const initialNames = { workflows: ['ci', 'lint', 'Release images and publish the container manifests for every platform'] }
const nameLoads: string[] = []
async function loadNames() {
  nameLoads.push('names')
  return initialNames
}
const olderRuns = [run(4, {}), run(5, { workflow_name: 'lint' })]
let nextRuns = initialRuns
const resolvers: (() => void)[] = []
const loads: string[] = []
const authorizeCalls: RepoParams[] = []
const repoSummary = (actor: string) => ({
  id: 'octo/demo', owner_handle: 'octo', name: 'demo', lifecycle_state: 'Ready', open_request_count: 0,
  views: [{ id: 'public', name: 'Public', includes: [], readers: 'anyone' as const }, { id: 'private', name: 'Private', includes: 'all' as const, readers: 'assigned' as const }],
  access: { actor, view: 'private' },
}) as RepoLiveState['repo']
const live = { repo: repoSummary('Owner') } as RepoLiveState
Object.assign(window, {
  authorizeCalls,
  loads,
  nameLoads,
  setNextRuns: (runs: GitHubWorkflowRunListResponse) => { nextRuns = runs },
  clearLoads: () => { loads.length = 0 },
  finishLoad: () => resolvers.shift()?.(),
  emitRunsChanged: () => invalidateRepoResources(repoResourceScope(live.repo, 'viewer'), {
    repo_id: 'octo/demo', incarnation_id: 'incarnation', version: 0, kind: 'GitHubWorkflowRunsChanged',
  }),
})
const subscribe = () => () => {}

function step(number: number, name: string, overrides: Partial<GitHubWorkflowStepResponse> = {}): GitHubWorkflowStepResponse {
  return {
    number, name, status: 'completed', conclusion: 'success',
    started_at_unix: now - 300 + number * 10, completed_at_unix: now - 295 + number * 10, ...overrides,
  }
}
function job(id: number, name: string, overrides: Partial<GitHubWorkflowJobResponse>): GitHubWorkflowJobResponse {
  return {
    id, name, status: 'completed', conclusion: 'success', started_at_unix: now - 300, completed_at_unix: now - 200,
    html_url: `https://github.com/octo/demo/actions/runs/1/job/${id}`, steps: [], ...overrides,
  }
}
const setUp = step(1, 'Set up job')
const checkout = step(2, 'Run actions/checkout@v4')
const runningJob = job(103, 'build', {
  status: 'in_progress', conclusion: null, completed_at_unix: null,
  steps: [setUp, checkout, step(3, 'Build the web bundle and the API image', { status: 'in_progress', conclusion: null, completed_at_unix: null }), step(4, 'Upload artifacts', { status: 'queued', conclusion: null, started_at_unix: null, completed_at_unix: null })],
})
const validate = 'Validate selected components'
const passedJob = (id: number, name: string) => job(id, name, { steps: [setUp, checkout, step(3, 'Run checks', { completed_at_unix: now - 200 })] })
let runDetail: GitHubWorkflowRunDetailResponse = {
  run: initialRuns.workflow_runs[0]!,
  jobs: [
    job(101, 'lint', { steps: [setUp, checkout, step(3, 'Install dependencies'), step(4, 'Lint', { completed_at_unix: now - 240 }), step(5, 'Complete job')] }),
    job(102, `${validate} / Server validation / test (ubuntu-latest, node 24)`, {
      conclusion: 'failure',
      steps: [setUp, checkout, step(3, 'Run the unit and integration test suites', { conclusion: 'failure', completed_at_unix: now - 230 }), step(4, 'Upload coverage', { conclusion: 'skipped' }), step(5, 'Complete job')],
    }),
    passedJob(106, `${validate} / Server validation / Web validation / Check scope-web components in a production build`),
    passedJob(107, `${validate} / Server validation / Server validation gate`),
    passedJob(108, `${validate} / Client validation / Check the command-line client on every supported platform and shell`),
    passedJob(109, `${validate} / Client validation / Check desktop`),
    passedJob(110, `${validate} / Integration validation / Signed-in contribution journey`),
    passedJob(111, `${validate} / Integration validation / Seeded local stack browser smoke`),
    runningJob,
    job(104, 'deploy preview environment to the staging cluster', {
      status: 'queued', conclusion: null, started_at_unix: null, completed_at_unix: null,
    }),
    job(105, 'revoke preview token', { conclusion: 'skipped', started_at_unix: null }),
  ],
  jobs_not_read_yet: false,
  opening_job_id: 102,
  opening_job_log: null,
}
const ESC = '\u001b'
const stamp = (offset: number) => `${new Date((now - 300 + offset) * 1000).toISOString().replace('Z', '0000Z')} `
const stamped = (offset: number, lines: string[]) => lines.map((line) => `${stamp(offset)}${line}`)
const testLog = [
  ...stamped(10, ['Current runner version: \'2.329.0\'', '##[group]Operating System', 'Ubuntu', '24.04.3', 'LTS', '##[endgroup]']),
  ...stamped(20, ['##[group]Run actions/checkout@v4', 'with:', '  repository: octo/demo', '##[endgroup]', 'Syncing repository: octo/demo']),
  ...stamped(30, [
    '##[group]Run pnpm test',
    'pnpm test',
    'shell: /usr/bin/bash -e {0}',
    '##[endgroup]',
    '##[command]/usr/bin/pnpm test',
    ...Array.from({ length: 40 }, (_, index) => `  ${ESC}[32m✔${ESC}[39m suite ${index} passes every case it was given, including the long-running integration fixtures ${ESC}[2m(${index * 3}ms)${ESC}[22m`),
    `  ${ESC}[31m× request queue keeps its order${ESC}[39m ${ESC}[2m(12ms)${ESC}[22m`,
    `    ${ESC}[1mAssertionError${ESC}[22m: expected ${ESC}[33m3${ESC}[39m runs, got ${ESC}[33m2${ESC}[39m`,
    `    at ${ESC}[36mrun-history-model.test.ts:41:10${ESC}[39m`,
    `${ESC}[38;5;244mtests 41 · pass 40 · fail 1${ESC}[0m`,
    '##[warning]Node.js 20 actions are deprecated.',
    '##[error]Process completed with exit code 1.',
  ]),
  ...stamped(50, ['Cleaning up orphan processes']),
]
const logs: Record<string, GitHubWorkflowJobLogResponse> = {
  101: { state: 'kept', text: `﻿${stamp(40)}##[group]Run pnpm lint\n${stamp(40)}$ oxlint src\n${stamp(40)}##[endgroup]\n${stamp(41)}Found 0 warnings and 0 errors.\n`, truncated: false, fetched: false },
  102: { state: 'kept', text: `﻿${testLog.join('\n')}\n`, truncated: true, fetched: false },
  103: { state: 'kept', text: `${stamp(35)}Build finished.\n`, truncated: false, fetched: false },
}
runDetail = { ...runDetail, opening_job_log: logs[102]! }
const longLogDetail: GitHubWorkflowRunDetailResponse = {
  run: initialRuns.workflow_runs[2]!,
  jobs: [job(301, 'package', { steps: [setUp, step(2, 'Package every artifact', { completed_at_unix: now - 200 })] })],
  jobs_not_read_yet: false,
  opening_job_id: 301,
  opening_job_log: {
    state: 'kept',
    text: Array.from({ length: 21_000 }, (_, index) => `${stamp(20)}packaged artifact ${index} into the release bundle`).join('\n'),
    truncated: false,
    fetched: false,
  },
}
const runLoads: string[] = []
const logLoads: string[] = []
const detailResolvers: (() => void)[] = []
Object.assign(window, {
  runLoads,
  logLoads,
  finishRunLoad: () => detailResolvers.shift()?.(),
  finishBuild: () => {
    const finished = { ...runningJob, status: 'completed' as const, conclusion: 'success' as const, completed_at_unix: now }
    runDetail = { ...runDetail, jobs: runDetail.jobs.map((listed) => listed.id === 103 ? finished : listed) }
  },
  startDeploy: () => {
    const deploying = step(2, 'Deploy the preview', { status: 'in_progress', conclusion: null, completed_at_unix: null })
    runDetail = {
      ...runDetail,
      jobs: runDetail.jobs.map((listed) => listed.id === 104
        ? { ...listed, status: 'in_progress' as const, started_at_unix: now - 290, steps: [setUp, deploying] }
        : listed),
    }
  },
  emitRunChanged: (githubRunId: number) => invalidateRepoResources(repoResourceScope(live.repo, 'viewer'), {
    repo_id: 'octo/demo', incarnation_id: 'incarnation', version: 0, kind: { GitHubWorkflowRunChanged: { github_run_id: githubRunId } },
  }),
})

async function loadRunDetail(runId: string) {
  runLoads.push(runId)
  await new Promise<void>((resolve) => detailResolvers.push(resolve))
  return runId === '3' ? longLogDetail : runDetail
}
async function loadJobLog(jobId: string) {
  logLoads.push(jobId)
  return logs[jobId] ?? { state: 'not_run' }
}

async function loadRuns({ after, workflow }: RepoGitHubWorkflowRunsInput) {
  loads.push(`${workflow ?? 'all'}${after ? ` after ${after}` : ''}`)
  await new Promise<void>((resolve) => resolvers.push(resolve))
  if (workflow) {
    const runs = [...nextRuns.workflow_runs, ...olderRuns].filter((listed) => listed.workflow_name === workflow)
    return { configured: true, github: { ...nextRuns, workflow_runs: runs, next_cursor: null } }
  }
  return {
    configured: true,
    github: after ? { ...nextRuns, workflow_runs: olderRuns, next_cursor: null } : nextRuns,
  }
}

const loadSettings = async () => ({
  collaboration: null,
  github: { configured: true, connection: null, required_checks: [], review_bots: [], can_confirm_public: true, can_publish_main: true, setup_check: null, run_import_count: 50, run_import: null },
})
async function startAuthorization(params: RepoParams) {
  authorizeCalls.push(params)
  return { authorize_url: '#github-authorize' }
}

function Repository() {
  const { owner, repo } = useParams({ strict: false })
  const [actor, setActor] = useState('Owner')
  Object.assign(window, { setActor })
  const state = { ...live, repo: repoSummary(actor) }
  return <RepoLayoutProvider live={state} subscribe={subscribe}>
    <RepoShell params={{ owner: owner!, repo: repo! }} repo={state.repo}><Outlet /></RepoShell>
  </RepoLayoutProvider>
}

function EmptyRuns() {
  const { owner, repo } = useParams({ strict: false })
  const configured = new URLSearchParams(location.search).get('configured') !== 'false'
  return <main className="px-4 pt-7">
    <RunsCiEmptyState
      github={{ configured, loadSettings, startAuthorization }}
      hasWorkflows={false}
      params={{ owner: owner!, repo: repo! }}
    />
  </main>
}

function ConnectedNoRuns() {
  const { owner, repo } = useParams({ strict: false })
  const none = { actions_url: 'https://github.com/octo/demo/actions', workflow_runs: [], next_cursor: null }
  return <GitHubWorkflowRunsPage
    initialNames={{ workflows: [] }}
    initialRuns={none}
    loadNames={async () => ({ workflows: [] })}
    loadRunDetail={loadRunDetail}
    loadRuns={async () => ({ configured: true, github: none })}
    params={{ owner: owner!, repo: repo! }}
  />
}

function Runs() {
  const { owner, repo } = useParams({ strict: false })
  return <GitHubWorkflowRunsPage
    initialNames={initialNames}
    initialRuns={initialRuns}
    loadNames={loadNames}
    loadRunDetail={loadRunDetail}
    loadRuns={loadRuns}
    params={{ owner: owner!, repo: repo! }}
  />
}

function Run() {
  const { owner, repo, runId } = useParams({ strict: false })
  const initialDetail = runId === '1' ? runDetail : runId === '3' ? null : {
    run: initialRuns.workflow_runs[1]!,
    jobs: [],
    jobs_not_read_yet: true,
    opening_job_id: null,
    opening_job_log: null,
  }
  return <GitHubWorkflowRunDetailPage
    initialDetail={initialDetail}
    initialScope={repoResourceScope(live.repo, 'viewer')}
    key={runId}
    loadDetail={() => loadRunDetail(runId!)}
    loadLog={loadJobLog}
    params={{ owner: owner!, repo: repo!, run_id: runId! }}
  />
}

function Request() {
  const { owner, repo, requestId } = useParams({ strict: false })
  return <div className="p-6">
    <h1>Request {requestId}</h1>
    <Link params={{ owner: owner!, repo: repo! }} to="/$owner/$repo/runs">Back to runs</Link>
  </div>
}

const root = createRootRoute({ component: Outlet })
const repository = createRoute({ getParentRoute: () => root, path: '$owner/$repo', component: Repository })
const routeTree = root.addChildren([repository.addChildren([
  createRoute({ getParentRoute: () => repository, path: '/', component: () => <p>Code</p> }),
  createRoute({ getParentRoute: () => repository, path: 'runs', component: Runs }),
  createRoute({ getParentRoute: () => repository, path: 'runs/$runId', component: Run }),
  createRoute({ getParentRoute: () => repository, path: 'runs-empty', component: EmptyRuns }),
  createRoute({ getParentRoute: () => repository, path: 'runs-connected', component: ConnectedNoRuns }),
  createRoute({ getParentRoute: () => repository, path: 'settings', component: () => <h1>Settings</h1> }),
  createRoute({ getParentRoute: () => repository, path: 'requests/$requestId', component: Request }),
])])
const router = createRouter({ routeTree })
createRoot(document.getElementById('root')!).render(<RouterProvider router={router} />)
