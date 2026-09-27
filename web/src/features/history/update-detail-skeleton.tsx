import { PanelState } from '@/components/empty-state'
import { FileSystemTreeSkeleton } from '@/components/file-system-tree'
import { FileWorkbench } from '@/components/file-workbench'
import { TextSkeleton } from '@/components/ui/skeleton'
import { useState } from 'react'

// Mirrors HistoryEntryDetailPanel: the update header, then its changed files
// beside an empty preview, because no diff opens until a file is chosen.
export function UpdateDetailSkeleton() {
  const [navigationOpen, setNavigationOpen] = useState(true)
  return (
    <div className="min-w-0">
      <div className="border-b border-border px-5 py-5 sm:px-6">
        <TextSkeleton length="long" size="title" />
        <TextSkeleton className="mt-1.5" length="medium" size="meta" />
      </div>
      <FileWorkbench
        navigationOpen={navigationOpen}
        onNavigationOpenChange={setNavigationOpen}
        selectedPath={null}
      >
        <FileSystemTreeSkeleton metaColumnLabel="change" />
        <PanelState><TextSkeleton length="medium" /></PanelState>
      </FileWorkbench>
    </div>
  )
}
