import { useState } from 'react'
import { createRoot } from 'react-dom/client'
import { createRootRoute, createRoute, createRouter, Link, Outlet, RouterProvider, useParams } from '@tanstack/react-router'
import { RepoShell } from '@/components/repo-shell'
import { RepoLayoutProvider } from '@/features/repo-detail/repo-layout-context'
import { invalidateRepoResources } from '@/features/repo-detail/repo-resource-invalidation'
import { repoResourceScope } from '@/features/repo-detail/repo-resource-scope'
import { GitHubWorkflowRunsPage } from '@/features/runs/github-workflow-runs'
import { RunsCiEmptyState } from '@/features/runs/runs-ci-empty-state'
import type { RepoGitHubWorkflowRunsInput, RepoParams } from '@/api/types'
import type { RepoLiveState } from '@/api/types'
import type { GitHubWorkflowRunListResponse, GitHubWorkflowRunResponse } from '@/api/types.generated'
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
  access: { actor },
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
  createRoute({ getParentRoute: () => repository, path: 'runs-empty', component: EmptyRuns }),
  createRoute({ getParentRoute: () => repository, path: 'runs-connected', component: ConnectedNoRuns }),
  createRoute({ getParentRoute: () => repository, path: 'settings', component: () => <h1>Settings</h1> }),
  createRoute({ getParentRoute: () => repository, path: 'requests/$requestId', component: Request }),
])])
const router = createRouter({ routeTree })
createRoot(document.getElementById('root')!).render(<RouterProvider router={router} />)
