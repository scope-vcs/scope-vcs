import { useState } from 'react'
import { createRoot } from 'react-dom/client'
import { createRootRoute, createRoute, createRouter, Link, Outlet, RouterProvider } from '@tanstack/react-router'
import { FixtureViewer } from '../request-workspace/clerk'
import { RepoLayoutProvider } from '@/features/repo-detail/repo-layout-context'
import { invalidateRepoResources } from '@/features/repo-detail/repo-resource-invalidation'
import { repoResourceScope } from '@/features/repo-detail/repo-resource-scope'
import { resetViewerState } from '@/lib/viewer-state'
import { RepositoryRunsRoute } from '@/features/runs/repository-runs-route'
import { RepositoryRunDetailPage } from '@/features/runs/repository-run-detail-page'
import type { RepoLiveState } from '@/api/types'
import type { RepoChangeEvent } from '@/api/types.generated'
import { cancelRun, detail, initialPage, loadDetail, loadLogs, seeded } from './actions'
import './styles.css'

const repoSummary = (actor: string) => ({ id: 'repo-1', owner_handle: 'owner', name: 'repo', access: { actor } }) as RepoLiveState['repo']
const params = { owner: 'owner', repo: 'repo' }
const initialScope = repoResourceScope(repoSummary('Owner'), 'adam')
const listeners = new Set<(event: RepoChangeEvent) => void>()
const subscribe = (listener: (event: RepoChangeEvent) => void) => { listeners.add(listener); return () => { listeners.delete(listener) } }
function Repository() {
  const [actor, setActor] = useState('Owner')
  const [viewer, setViewer] = useState('adam')
  const live = { repo: repoSummary(actor) } as RepoLiveState
  Object.assign(window, {
    setActor,
    setViewer: (value: string) => { resetViewerState(); setViewer(value) },
    emitChange: (kind: RepoChangeEvent['kind']) => {
      const event = { repo_id: 'repo-1', incarnation_id: 'incarnation', version: 1, kind }
      invalidateRepoResources(repoResourceScope(live.repo, viewer), event)
      listeners.forEach((listener) => listener(event))
    },
  })
  return <FixtureViewer value={viewer}><RepoLayoutProvider live={live} subscribe={subscribe}>
    <nav><Link to="/owner/repo/runs">History</Link> <Link to="/owner/repo/runs/run-1">Detail</Link> <Link to="/owner/repo/away">Away</Link></nav><Outlet />
  </RepoLayoutProvider></FixtureViewer>
}
const root = createRootRoute({ component: Outlet })
const repo = createRoute({ getParentRoute: () => root, path: 'owner/repo', component: Repository })
const runs = createRoute({ getParentRoute: () => repo, path: 'runs', component: () => <RepositoryRunsRoute
  initialResources={seeded ? { scope: initialScope, resources: initialPage } : null} params={params} /> })
const runDetail = createRoute({ getParentRoute: () => repo, path: 'runs/run-1', component: () => <RepositoryRunDetailPage
  initialDetail={seeded ? detail : null} initialScope={initialScope} loadDetail={loadDetail} loadLogs={loadLogs}
  params={{ ...params, run_id: 'run-1' }} cancelRun={cancelRun} retryRun={async () => {}} /> })
const away = createRoute({ getParentRoute: () => repo, path: 'away', component: () => <p>Code</p> })
createRoot(document.getElementById('root')!).render(<RouterProvider router={createRouter({ routeTree: root.addChildren([repo.addChildren([runs, runDetail, away])]) })} />)
