import { useState, type ReactNode } from 'react'
import { PendingSurface } from '@/components/pending-surface'
import { BlockSkeleton } from '@/components/ui/skeleton'
import { cn } from '@/lib/utils'
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
          <aside
            className={cn(
              'request-workspace-sidebar',
              collapsed && 'request-workspace-sidebar--collapsed',
            )}
          >
            {collapsed ? (
              <BlockSkeleton className="mx-auto mt-4 size-8" />
            ) : (
              <>
                <div className="request-workspace-sidebar-tools">
                  <BlockSkeleton className="h-8 w-full" />
                </div>
                <div className="px-2">
                  <RequestWorkspaceListSkeleton />
                </div>
                {['Unclaimed', 'Set aside'].map((label) => (
                  <div
                    className="mx-2 rounded-md px-3 py-2 text-xs font-medium text-muted-foreground"
                    key={label}
                  >
                    {label}
                  </div>
                ))}
              </>
            )}
          </aside>
        }
      >
        {children}
      </RequestWorkspaceShell>
    </PendingSurface>
  )
}
