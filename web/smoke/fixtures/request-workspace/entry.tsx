import { useState } from 'react'
import { createRoot } from 'react-dom/client'
import { createRootRoute, createRoute, createRouter, Link, Outlet, RouterProvider, useParams } from '@tanstack/react-router'
import { RepoShell } from '@/components/repo-shell'
import { RepoLayoutProvider } from '@/features/repo-detail/repo-layout-context'
import { RequestWorkspaceSidebar } from '@/features/requests/request-workspace-sidebar'
import { RequestWorkspaceShell } from '@/features/requests/request-workspace-shell'
import type { RepoLiveState } from '@/api/types'
import type { RequestQueueItemResponse } from '@/api/types.generated'
import { FixtureViewer } from './clerk'
import './styles.css'

const titles = [
  'Let an invite have many links and tell each visitor where they stand',
  'Stop unevaluated request heads from merging',
  'Request without available actions',
]
const items = titles.map((title, index) => ({
  request: { id: `request-${index}`, title },
  author: { handle: 'adam' },
  attention: { reason: 'authored', can_set_aside: index < 2 },
  attention_at_unix: Math.floor(Date.now() / 1000) - index * 480,
})) as RequestQueueItemResponse[]
const empty = { requests: [], next_cursor: null, next_attention_at_unix: null }
const pages = { active: { ...empty, requests: items }, unclaimed: empty, set_aside: empty, done: empty }
const subscribe = () => () => {}
const noop = () => {}

function Repository() {
  const { owner, repo } = useParams({ strict: false })
  const [viewer, setViewer] = useState('adam')
  const [actor, setActor] = useState('Owner')
  Object.assign(window, { setViewer, setActor })
  const live = { repo: {
    id: `${owner}/${repo}`, owner_handle: owner, name: repo, lifecycle_state: 'Ready',
    open_request_count: items.length, access: { actor },
  } } as RepoLiveState
  return <FixtureViewer value={viewer}>
    <RepoLayoutProvider live={live} subscribe={subscribe}>
      <RepoShell params={{ owner: owner!, repo: repo! }}><Outlet /></RepoShell>
    </RepoLayoutProvider>
  </FixtureViewer>
}

function Workspace() {
  const { owner, repo, requestId } = useParams({ strict: false })
  const [collapsed, setCollapsed] = useState(false)
  return <RequestWorkspaceShell collapsed={collapsed} detailOpenOnMobile={Boolean(requestId)}
    onCollapsedChange={setCollapsed} sidebar={
      <RequestWorkspaceSidebar pages={pages} collapsed={collapsed} onCollapsedChange={setCollapsed}
        query="" onSearch={noop} loading={false} skeleton={false} error={null} actionError={null}
        maintainer onRetry={noop} onLoadMore={noop} onAction={noop}
        params={{ owner: owner!, repo: repo! }} pendingId={null} selectedId={requestId}
        focus={false} onFocusToggle={noop} />
    }><Outlet /></RequestWorkspaceShell>
}

function Request() {
  const { owner, repo, requestId } = useParams({ strict: false })
  const params = { owner: owner!, repo: repo!, requestId: requestId! }
  return <div className="p-6">
    <h1>{titles[Number(requestId!.split('-').at(-1))]}</h1>
    <nav className="flex gap-4 py-4" aria-label="Request views">
      <Link to="/$owner/$repo/requests/$requestId" params={params}>Discussion</Link>
      <Link to="/$owner/$repo/requests/$requestId/changes" params={params}
        search={{ file: 'src/main.ts', revision: 'revision-1' }} hash="line-12">Changes</Link>
      <Link to="/$owner/$repo/requests/$requestId/details" params={params}>Details</Link>
    </nav>
    <Outlet />
  </div>
}

const root = createRootRoute({ component: Outlet })
const repository = createRoute({ getParentRoute: () => root, path: '$owner/$repo', component: Repository })
const requests = createRoute({ getParentRoute: () => repository, path: 'requests', component: Workspace })
const request = createRoute({ getParentRoute: () => requests, path: '$requestId', component: Request })
const routeTree = root.addChildren([repository.addChildren([
  createRoute({ getParentRoute: () => repository, path: '/', component: () => <p>Code</p> }),
  createRoute({ getParentRoute: () => repository, path: 'runs', component: () => <h1>Runs</h1> }),
  requests.addChildren([
    createRoute({ getParentRoute: () => requests, path: '/', component: () => <p>Select a request</p> }),
    request.addChildren([
      createRoute({ getParentRoute: () => request, path: '/', component: () => <p>Discussion content</p> }),
      createRoute({ getParentRoute: () => request, path: 'changes', validateSearch: (search) => search,
        component: () => <p>Changes content</p> }),
      createRoute({ getParentRoute: () => request, path: 'details', component: () => <p>Details content</p> }),
    ]),
  ]),
])])
const router = createRouter({ routeTree })
Object.assign(window, { navigate: (to: string) => router.navigate({ to }) })
createRoot(document.getElementById('root')!).render(<RouterProvider router={router} />)
