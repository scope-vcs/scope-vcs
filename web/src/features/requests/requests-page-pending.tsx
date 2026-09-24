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

// The real sidebar with no queue yet. Whether the viewer maintains the
// repository arrives with it, so the sidebar cannot pick its groups.
export function RequestsPagePending({ children }: { children?: ReactNode }) {
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
            maintainer={null}
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
