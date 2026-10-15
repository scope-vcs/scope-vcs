import { ApplicationTopbar } from '@/components/application-topbar'
import { AppShell } from '@/components/app-shell'
import { PageRail } from '@/components/page-header'
import { PendingSurface } from '@/components/pending-surface'
import type { ReactNode } from 'react'

export function ApplicationPendingShell({
  children,
  contextLabel,
  label,
}: {
  children?: ReactNode
  contextLabel?: string
  label: string
}) {
  return (
    <AppShell header={() => <ApplicationTopbar contextLabel={contextLabel} />}>
      <PageRail className="min-h-full">
        <PendingSurface label={label}>{children}</PendingSurface>
      </PageRail>
    </AppShell>
  )
}
