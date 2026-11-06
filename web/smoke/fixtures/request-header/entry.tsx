import { createRoot } from 'react-dom/client'
import { createRootRoute, createRouter, Link, RouterProvider } from '@tanstack/react-router'
import { useRef, useState } from 'react'
import { Code, SlidersHorizontal } from 'lucide-react'
import { Button } from '@/components/ui/button'
import type { RequestQueueItemResponse, RequestSummaryResponse } from '@/api/types.generated'
import { RequestDetailHeader } from '@/features/requests/request-detail-header'
import { RequestChangesMenu } from '@/features/requests/request-changes-menu'
import { RequestLifecycleActions } from '@/features/requests/request-lifecycle-actions'
import { RequestMoreMenu } from '@/features/requests/request-more-menu'
import { RequestWorkspaceProvider } from '@/features/requests/request-workspace-context'
import '@/features/requests/request-detail.css'
import '../shared/styles.css'

const params = { owner: 'adamblumoff', repo: 'scope-vcs', requestId: 'request' }
const request = {
  id: 'request', name: 'repository-row-security', title: 'Enforce repository access',
  head_oid: 'a'.repeat(40), view: 'private', state: 'Open', author_role: 'Member',
  base: { kind: 'main', review_base_oid: 'b'.repeat(40) }, base_version: 1,
  stack_status: 'main', stack_depth: 1, submitted_at_unix: 1793707200,
  mergeability: { status: 'ReviewFindingsOpen', reason: 'Review findings remain open.' },
  permissions: { can_merge: true, can_close: true, can_submit: false, can_view_activity: true },
} as RequestSummaryResponse
const activity = { identity: 'header-activity', version: '1', load: async () => ({ events: [], next_cursor: null }) }
const actions = { error: null, pending: null, run: async () => false }
const autoMerge = {
  error: null, pending: null, authorize: async () => true, cancel: async () => true,
  status: {
    can_cancel: false, can_enable: true, head_oid: request.head_oid, intent: null,
    request_id: request.id, revision_id: 'revision', waiting_reason: request.mergeability.reason,
  },
}

function App() {
  const [name, setName] = useState(request.name)
  const more = useRef<HTMLButtonElement | null>(null)
  Object.assign(window, { setBranch: setName })
  return <main className="request-detail-pane">
    <RequestWorkspaceProvider value={{
      selected: { author: { handle: 'adamblumoff' }, claimer: null } as RequestQueueItemResponse,
      claim: () => {}, release: () => {},
    }}>
      <RequestDetailHeader
        params={params} request={{ ...request, name }} viewName="Maintainer"
        diffStat={{ additions: 5495, deletions: 2578, changed_files: 203 }}
        backLink={<a href="#requests">Requests</a>}
        actions={<>
          <Button asChild size="sm" variant="secondary"><Link to="/$owner/$repo/code" params={params}><Code />Browse code</Link></Button>
          <RequestChangesMenu activity={activity} params={params} />
          <Button className="request-details-trigger" size="sm" variant="secondary"><SlidersHorizontal />Details</Button>
          <RequestLifecycleActions actions={actions} autoMerge={autoMerge} request={request} viewerId="viewer"
            className="fixed inset-x-0 bottom-0 z-30 justify-end border-t border-border bg-background px-(--app-inset) py-3 min-[701px]:static min-[701px]:border-0 min-[701px]:p-0" />
          <RequestMoreMenu actions={actions} disabled={false} onViewActivity={() => {}} request={request} triggerRef={more} />
        </>}
      />
    </RequestWorkspaceProvider>
  </main>
}

const root = createRootRoute({ component: App })
createRoot(document.getElementById('root')!).render(<RouterProvider router={createRouter({ routeTree: root })} />)
