import { useState } from 'react'
import { createRoot } from 'react-dom/client'
import { createRootRoute, createRoute, createRouter, Outlet, RouterProvider } from '@tanstack/react-router'
import type { RequestDetailResponse, RequestRevisionListResponse, RequestStackStatus, RequestSummaryResponse } from '@/api/types.generated'
import { RequestStackContext } from '@/features/requests/request-stack-context'
import { RequestChangesWorkbench, type RequestChangesSearch } from '@/features/requests/request-changes-workbench'
import { RequestLifecycleActions } from '@/features/requests/request-lifecycle-actions'
import { RepoLayoutProvider } from '@/features/repo-detail/repo-layout-context'
import type { RepoLiveState } from '@/api/types'
import './styles.css'

const head = 'b'.repeat(40)
const base = 'a'.repeat(40)
const files = [{ path: '/src/child.ts', kind: 'Added', old_oid: null, new_oid: head, old_mode: null, new_mode: '100644', label: 'private' }]
const revision = {
  base: { kind: 'request', request_id: 'parent', revision_id: 'parent-revision', head_oid: base }, base_version: 1,
  id: 'child-revision', position: 2, actor: null, old_head_oid: base, new_head_oid: head,
  inspection: 'Complete', created_at_unix: 1,
  commits: [{ oid: head, parent_oids: [base], author: 'adam', message: 'Add child UI', change_count: 1, files_truncated: false, files }],
} as RequestRevisionListResponse['revisions'][number]
const calls: unknown[] = []

function Stack() {
  const view = new URL(location.href).searchParams.get('view') ?? 'private'
  const isPrivate = view === 'private'
  const visibleFiles = files.map((file) => ({ ...file, label: isPrivate ? 'private' : 'public' }))
  const [opaque, setOpaque] = useState(false)
  const [stackStatus, setStackStatus] = useState<RequestStackStatus>(isPrivate ? 'waiting_for_parent' : 'main')
  const [search, setSearch] = useState<RequestChangesSearch>({ revision: revision.id })
  Object.assign(window, { setStackStatus, setOpaque, calls })
  const requestBase = isPrivate ? revision.base : { kind: 'main' as const, review_base_oid: base }
  const status = stackStatus === 'main' ? 'Ready' : stackStatus === 'parent_rewritten' ? 'ParentRewritten'
    : stackStatus === 'parent_closed' ? 'ParentClosed'
    : stackStatus === 'ancestor_blocked' ? 'AncestorBlocked'
    : stackStatus === 'integration_conflict' ? 'IntegrationConflict' : 'WaitingForParent'
  const request = {
    id: 'child', title: 'Child UI', author_role: 'Member', head_oid: head, view, stack_status: stackStatus, stack_depth: isPrivate ? 2 : 1, base_version: 1,
    base: requestBase,
    permissions: { can_merge: true, can_submit: false }, mergeability: { status, reason: status },
  } as RequestSummaryResponse
  const parent = { id: 'parent', name: 'parent', title: 'Parent foundation', state: 'Open', stack_status: 'main', stack_depth: 1, current_revision_id: 'current-parent-revision', head_oid: 'c'.repeat(40) }
  const child = { ...parent, id: 'grandchild', name: 'grandchild', title: 'Follow-up child', stack_depth: 3 }
  const detail = { request, stack_navigation: { parent: opaque ? { kind: 'unavailable' } : { kind: 'available', request: parent }, ancestors: opaque ? [] : [parent], ancestors_unavailable: opaque, children: { requests: [child], next_cursor: null } } } as RequestDetailResponse
  const mode = search.comparison ?? (isPrivate ? 'request' : 'push')
  const revisions = {
    revisions: [{ ...revision, base: requestBase, commits: revision.commits.map((commit) => ({ ...commit, files: isPrivate && mode === 'commit' ? [] : visibleFiles, files_truncated: isPrivate && mode === 'commit' })) }], review_revision_id: revision.id, has_earlier_revisions: false,
    comparison: isPrivate ? { revision_id: revision.id, mode, base_oid: base, head_oid: head, inspection: 'Complete', files: visibleFiles } : null,
  } as RequestRevisionListResponse
  return <RepoLayoutProvider live={{ repo: { views: [{ id: view, name: view, readers: view === 'public' ? 'anyone' : 'assigned', includes: isPrivate ? 'all' : [] }] } } as RepoLiveState} subscribe={() => () => {}}>
  <>
    {isPrivate ? <RequestStackContext detail={detail} params={{ owner: 'adam', repo: 'demo' }} /> : null}
    <div className="p-3">
      <RequestLifecycleActions
        request={request} viewerId="adam"
        actions={{ error: null, pending: null, run: async (command) => { calls.push(command); return false } }}
        autoMerge={{ error: null, pending: null, status: { can_enable: false, can_cancel: false, intent: null, request_id: 'child', revision_id: revision.id, head_oid: head, waiting_reason: status }, authorize: async () => false, cancel: async () => false }}
      />
    </div>
    <RequestChangesWorkbench
      accessScope={`fixture:${view}:adam`} allowRequestComparison={isPrivate} view={view} repoId="repo" params={{ owner: 'adam', repo: 'demo', request_id: 'child' }}
      initialDiscussionReferences={{ commitKey: null, page: null }} revisions={revisions} search={search}
      onSearchChange={setSearch}
      loadDiscussions={async () => ({ discussions: [], next_cursor: null, snapshot_version: 0 })}
      loadDiff={async (input) => {
        calls.push(input)
        return { kind: 'Added', new_mode: '100644', old_mode: null, path: input.path, presentation: { kind: 'empty' } }
      }}
    />
  </>
  </RepoLayoutProvider>
}

const root = createRootRoute({ component: Outlet })
const request = createRoute({ getParentRoute: () => root, path: '/$owner/$repo/requests/$requestId', component: Stack })
const changes = createRoute({ getParentRoute: () => root, path: '/$owner/$repo/requests/$requestId/changes', component: Stack })
const router = createRouter({ routeTree: root.addChildren([request, changes]) })
createRoot(document.getElementById('root')!).render(<RouterProvider router={router} />)
