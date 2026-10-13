import { PanelState } from '@/components/empty-state'
import { FileSystemTreeSkeleton } from '@/components/file-system-tree'
import { FileWorkbench } from '@/components/file-workbench'
import { TextSkeleton } from '@/components/ui/skeleton'
import { useState } from 'react'

export function UpdateDetailSkeleton() {
  const [navigationOpen, setNavigationOpen] = useState(true)
  return (
    <div className="min-w-0">
      <div className="border-b border-border px-(--app-inset) py-5">
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
