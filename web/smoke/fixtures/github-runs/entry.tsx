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
import './styles.css'

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
  workflows: ['ci', 'lint', 'Release images and publish the container manifests for every platform'],
  next_cursor: 'page-2',
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
  setNextRuns: (runs: GitHubWorkflowRunListResponse) => { nextRuns = runs },
  clearLoads: () => { loads.length = 0 },
  finishLoad: () => resolvers.shift()?.(),
  emitRunsChanged: () => invalidateRepoResources(repoResourceScope(live.repo, 'adam'), {
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
let runDetail: GitHubWorkflowRunDetailResponse = {
  run: initialRuns.workflow_runs[0]!,
  jobs: [
    job(101, 'lint', { steps: [setUp, checkout, step(3, 'Install dependencies'), step(4, 'Lint'), step(5, 'Complete job')] }),
    job(102, 'test (ubuntu-latest, node 24)', {
      conclusion: 'failure',
      steps: [setUp, checkout, step(3, 'Run the unit and integration test suites', { conclusion: 'failure' }), step(4, 'Upload coverage', { conclusion: 'skipped' }), step(5, 'Complete job')],
    }),
    runningJob,
    job(104, 'deploy preview environment to the staging cluster', {
      status: 'queued', conclusion: null, started_at_unix: null, completed_at_unix: null,
    }),
    job(105, 'revoke preview token', { conclusion: 'skipped', started_at_unix: null }),
  ],
  jobs_unavailable: null,
}
const stamp = '2026-10-05T12:00:00.1234567Z '
const logs: Record<string, GitHubWorkflowJobLogResponse> = {
  101: { state: 'kept', text: `\uFEFF${stamp}##[group]Run pnpm lint\n${stamp}$ oxlint src\n${stamp}Found 0 warnings and 0 errors.\n`, truncated: false },
  102: {
    state: 'kept',
    text: Array.from({ length: 40 }, (_, index) => `${stamp}  ✔ suite ${index} passes every case it was given, including the long-running integration fixtures (${index * 3}ms)`).join('\n')
      + `\n${stamp}  ✖ request queue keeps its order (12ms)\n${stamp}##[error]Process completed with exit code 1.\n`,
    truncated: true,
  },
  103: { state: 'kept', text: `${stamp}Build finished.\n`, truncated: false },
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
  emitRunChanged: (githubRunId: number) => invalidateRepoResources(repoResourceScope(live.repo, 'adam'), {
    repo_id: 'octo/demo', incarnation_id: 'incarnation', version: 0, kind: { GitHubWorkflowRunChanged: { github_run_id: githubRunId } },
  }),
})

async function loadRunDetail(runId: string) {
  runLoads.push(runId)
  await new Promise<void>((resolve) => detailResolvers.push(resolve))
  return runDetail
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
  github: { configured: true, connection: null, required_checks: [], can_confirm_public: true, setup_check: null, run_import_count: 50, run_import: null },
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
  const none = { actions_url: 'https://github.com/octo/demo/actions', workflow_runs: [], workflows: [], next_cursor: null }
  return <GitHubWorkflowRunsPage
    initialRuns={none}
    loadRuns={async () => ({ configured: true, github: none })}
    params={{ owner: owner!, repo: repo! }}
  />
}

function Runs() {
  const { owner, repo } = useParams({ strict: false })
  return <GitHubWorkflowRunsPage initialRuns={initialRuns} loadRuns={loadRuns} params={{ owner: owner!, repo: repo! }} />
}

function Run() {
  const { owner, repo, runId } = useParams({ strict: false })
  const initialDetail = runId === '1' ? runDetail : {
    run: initialRuns.workflow_runs[1]!,
    jobs: [],
    jobs_unavailable: 'The jobs could not be read. They appear once the repository can be read again.',
  }
  return <GitHubWorkflowRunDetailPage
    initialDetail={initialDetail}
    initialScope={repoResourceScope(live.repo, 'adam')}
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
