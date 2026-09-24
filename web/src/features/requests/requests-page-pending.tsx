import { useState, type ReactNode } from 'react'
import { PendingSurface } from '@/components/pending-surface'
import { BlockSkeleton } from '@/components/ui/skeleton'
import { RequestWorkspaceListSkeleton } from './request-workspace-list'
import { RequestWorkspaceShell } from './request-workspace-shell'

export function RequestsPagePending({ children }: { children?: ReactNode }) {
  const [collapsed, setCollapsed] = useState(false)
  return (
    <PendingSurface label="Loading requests">
      <RequestWorkspaceShell
        collapsed={collapsed}
        detailOpenOnMobile={Boolean(children)}
        onCollapsedChange={setCollapsed}
        sidebar={
          <aside className="request-workspace-sidebar" data-state={collapsed ? 'closed' : 'pinned'}>
            <div className="request-workspace-sidebar-inner">
              <div className="request-workspace-sidebar-tools">
                <BlockSkeleton className="h-8 w-full" />
              </div>
              <h2 className="request-workspace-group-label text-foreground">
                <span>Needs you</span>
              </h2>
              <RequestWorkspaceListSkeleton />
              <div className="request-workspace-disclosures">
                {['Waiting on others', 'Unclaimed', 'Set aside', 'Done'].map((label) => (
                  <div className="request-workspace-group-label text-muted-foreground" key={label}>
                    <span>{label}</span>
                  </div>
                ))}
              </div>
            </div>
          </aside>
        }
      >
        {children}
      </RequestWorkspaceShell>
    </PendingSurface>
  )
}
