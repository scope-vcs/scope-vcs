import { useState, type ReactNode } from 'react'
import { PendingSurface } from '@/components/pending-surface'
import { BlockSkeleton, TextSkeleton } from '@/components/ui/skeleton'
import { RequestWorkspaceListSkeleton } from './request-workspace-list'
import {
  readRequestWorkspaceCollapsed,
  saveRequestWorkspaceCollapsed,
} from './request-workspace-collapse'
import { RequestWorkspaceShell } from './request-workspace-shell'

export function RequestsPagePending({ children }: { children?: ReactNode }) {
  const [collapsed, setCollapsed] = useState(readRequestWorkspaceCollapsed)
  return (
    <PendingSurface label="Loading requests">
      <RequestWorkspaceShell
        collapsed={collapsed}
        detailOpenOnMobile={Boolean(children)}
        onCollapsedChange={(value) => {
          setCollapsed(value)
          saveRequestWorkspaceCollapsed(value)
        }}
        sidebar={
          <aside className="request-workspace-sidebar" data-state={collapsed ? 'closed' : 'pinned'}>
            {collapsed ? (
              <BlockSkeleton className="mt-[14px] ml-[11px] size-8" />
            ) : (
              <div className="request-workspace-sidebar-inner">
                <div className="request-workspace-sidebar-tools">
                  <BlockSkeleton className="h-8 w-full" />
                </div>
                {/* Which groups show depends on the viewer's access, which
                    arrives with the repository, so the label waits too. */}
                <div className="request-workspace-group-label">
                  <TextSkeleton length="short" size="meta" />
                  <TextSkeleton className="ml-auto" length="tiny" size="meta" />
                </div>
                <RequestWorkspaceListSkeleton />
              </div>
            )}
          </aside>
        }
      >
        {children}
      </RequestWorkspaceShell>
    </PendingSurface>
  )
}
