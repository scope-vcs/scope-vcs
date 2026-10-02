import { createRoot } from 'react-dom/client'
import { createRootRoute, createRoute, createRouter, Link, Outlet, RouterProvider, useParams } from '@tanstack/react-router'
import { RepoShell } from '@/components/repo-shell'
import { RepoLayoutProvider } from '@/features/repo-detail/repo-layout-context'
import { invalidateRepoResources } from '@/features/repo-detail/repo-resource-invalidation'
import { repoResourceScope } from '@/features/repo-detail/repo-resource-scope'
import { GitHubWorkflowRunsPage } from '@/features/runs/github-workflow-runs'
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
}
let nextRuns = initialRuns
const resolvers: (() => void)[] = []
const loads: string[] = []
const live = { repo: {
  id: 'octo/demo', owner_handle: 'octo', name: 'demo', lifecycle_state: 'Ready', open_request_count: 0,
  access: { actor: 'Owner' },
} } as RepoLiveState
Object.assign(window, {
  loads,
  setNextRuns: (runs: GitHubWorkflowRunListResponse) => { nextRuns = runs },
  finishLoad: () => resolvers.shift()?.(),
  emitRunsChanged: () => invalidateRepoResources(repoResourceScope(live.repo, 'adam'), {
    repo_id: 'octo/demo', incarnation_id: 'incarnation', version: 0, kind: 'GitHubWorkflowRunsChanged',
  }),
})
const subscribe = () => () => {}

async function loadRuns() {
  loads.push('load')
  await new Promise<void>((resolve) => resolvers.push(resolve))
  return { github: nextRuns }
}

function Repository() {
  const { owner, repo } = useParams({ strict: false })
  return <RepoLayoutProvider live={live} subscribe={subscribe}>
    <RepoShell params={{ owner: owner!, repo: repo! }} repo={live.repo}><Outlet /></RepoShell>
  </RepoLayoutProvider>
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
  createRoute({ getParentRoute: () => repository, path: 'requests/$requestId', component: Request }),
])])
const router = createRouter({ routeTree })
createRoot(document.getElementById('root')!).render(<RouterProvider router={router} />)
