import { useAuth } from '@clerk/tanstack-react-start'
import { useParams } from '@tanstack/react-router'
import { useState, type ReactNode } from 'react'
import { PendingSurface } from '@/components/pending-surface'
import {
  readRequestWorkspaceCollapsed,
  saveRequestWorkspaceCollapsed,
} from './request-workspace-collapse'
import { RequestWorkspaceShell } from './request-workspace-shell'
import { RequestWorkspaceSidebar } from './request-workspace-sidebar'

const ignore = () => {}

// The real sidebar with no queue yet. Whether a signed-in viewer maintains
// the repository arrives with it, so their sidebar cannot pick its groups yet.
// Signed-out viewers never maintain one and get the reader's groups at once.
export function RequestsPagePending({ children }: { children?: ReactNode }) {
  const { isSignedIn } = useAuth()
  const params = useParams({ from: '/$owner/$repo' })
  const selectedId = useParams({ strict: false, select: (value) => value.requestId })
  const [collapsed, setCollapsed] = useState(readRequestWorkspaceCollapsed)
  const changeCollapsed = (value: boolean) => {
    setCollapsed(value)
    saveRequestWorkspaceCollapsed(value)
  }
  return (
    <PendingSurface label="Loading requests">
      <RequestWorkspaceShell
        collapsed={collapsed}
        detailOpenOnMobile={Boolean(children)}
        onCollapsedChange={changeCollapsed}
        sidebar={
          <RequestWorkspaceSidebar
            actionError={null}
            collapsed={collapsed}
            error={null}
            focus={false}
            loading={false}
            maintainer={isSignedIn === false ? false : null}
            onAction={ignore}
            onCollapsedChange={changeCollapsed}
            onFocusToggle={ignore}
            onLoadMore={ignore}
            onRetry={ignore}
            onSearch={ignore}
            pages={undefined}
            params={params}
            pendingId={null}
            query=""
            selectedId={selectedId}
            skeleton
          />
        }
      >
        {children}
      </RequestWorkspaceShell>
    </PendingSurface>
  )
}
