import { useState, type ReactNode } from 'react'
import { PendingSurface } from '@/components/pending-surface'
import { BlockSkeleton, TextSkeleton } from '@/components/ui/skeleton'
import { cn } from '@/lib/utils'
import { RequestWorkspaceShell } from './request-workspace-shell'

const PENDING_ROWS = [{ id: 'first', length: 'long' }, { id: 'second', length: 'medium' }, { id: 'third', length: 'long' }] as const

export function RequestsPagePending({ children }: { children?: ReactNode }) {
  const [collapsed, setCollapsed] = useState(false)
  return (
    <PendingSurface label="Loading requests">
      <RequestWorkspaceShell collapsed={collapsed} detailOpenOnMobile={Boolean(children)} onCollapsedChange={setCollapsed} sidebar={(
        <aside className={cn('request-workspace-sidebar', collapsed && 'request-workspace-sidebar--collapsed')}>
          {collapsed ? <BlockSkeleton className="mx-auto mt-4 size-8" /> : (
            <>
              <div className="request-workspace-sidebar-tools"><BlockSkeleton className="h-8 w-full" /></div>
              {PENDING_ROWS.map(({ id, length }) => (
                <div className="mx-2 px-3 py-4" key={id}>
                  <TextSkeleton length={length} />
                  <TextSkeleton className="mt-3" length="short" size="meta" />
                </div>
              ))}
              {['Unclaimed', 'Set aside'].map((label) => <div className="request-workspace-shelf-toggle mx-2 w-auto" key={label}>{label}</div>)}
            </>
          )}
        </aside>
      )}>
        {children}
      </RequestWorkspaceShell>
    </PendingSurface>
  )
}
