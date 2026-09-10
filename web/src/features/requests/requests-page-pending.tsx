import type { ReactNode } from 'react'
import { PendingSurface } from '@/components/pending-surface'
import { BlockSkeleton, TextSkeleton } from '@/components/ui/skeleton'
import { RequestWorkspaceShell } from './request-workspace-sidebar'

const PENDING_ROWS = [{ id: 'first', length: 'long' }, { id: 'second', length: 'medium' }, { id: 'third', length: 'long' }] as const

export function RequestsPagePending({ children }: { children?: ReactNode }) {
  return (
    <PendingSurface label="Loading requests">
      <RequestWorkspaceShell collapsed={false} detailOpenOnMobile={Boolean(children)} sidebar={(
        <aside className="request-workspace-sidebar">
          <div className="border-b border-border p-4"><BlockSkeleton className="h-9 w-full" /></div>
          {PENDING_ROWS.map(({ id, length }) => (
            <div className="border-b border-border px-5 py-4" key={id}>
              <TextSkeleton length={length} />
              <TextSkeleton className="mt-3" length="short" size="meta" />
            </div>
          ))}
          {['Unclaimed', 'Set aside'].map((label) => <div className="border-b border-border px-5 py-4 text-xs text-muted-foreground" key={label}>{label}</div>)}
        </aside>
      )}>
        {children}
      </RequestWorkspaceShell>
    </PendingSurface>
  )
}
