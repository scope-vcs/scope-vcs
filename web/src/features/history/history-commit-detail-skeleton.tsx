import { FileSystemTreeSkeleton } from '@/components/file-system-tree'
import { FileWorkbench } from '@/components/file-workbench'
import { BlockSkeleton, TextSkeleton } from '@/components/ui/skeleton'
import { DiffDrawerSkeleton } from '@/features/review/diff-skeleton'
import { useState } from 'react'

// Mirrors HistoryEntryDetailPanel: the entry header, then its changed files
// beside the diff of the first one, which the page opens by default.
export function CommitDetailSkeleton() {
  const [navigationOpen, setNavigationOpen] = useState(false)
  return (
    <div className="min-w-0">
      <div className="border-b border-border px-5 py-4 sm:px-6">
        <div className="flex items-start gap-2">
          <BlockSkeleton className="h-5 w-11 shrink-0 rounded-md" />
          <TextSkeleton length="long" />
        </div>
        <TextSkeleton className="mt-1.5" length="medium" size="meta" />
        <TextSkeleton className="mt-2" length="short" size="meta" />
      </div>
      <FileWorkbench
        navigationOpen={navigationOpen}
        onNavigationOpenChange={setNavigationOpen}
        selectedPath={null}
      >
        <FileSystemTreeSkeleton metaColumnLabel="change" />
        <div className="h-[70vh] min-h-[340px] max-h-[720px] min-w-0 overflow-hidden">
          <DiffDrawerSkeleton />
        </div>
      </FileWorkbench>
    </div>
  )
}
