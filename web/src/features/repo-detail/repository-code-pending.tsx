import { FileWorkbench } from '@/components/file-workbench'
import { WorkbenchBar, WorkbenchPane } from '@/components/page-header'
import { PendingSurface } from '@/components/pending-surface'
import { BlockSkeleton, TextSkeleton } from '@/components/ui/skeleton'
import { useState } from 'react'
import {
  FileNavigatorSkeleton,
  SourceCodeSkeleton,
  SourceTabStripSkeleton,
} from './repository-code-skeletons'
import { RepositoryLatestActivityPending } from './repository-latest-activity'

// Mirrors RepositoryCodeView before its files arrive, without importing it:
// pending components load with the route tree, and the view's renderers
// would put the whole code page in the first download.
export function RepositoryCodePending() {
  const [navigationOpen, setNavigationOpen] = useState(false)
  return (
    <WorkbenchPane>
      <WorkbenchBar
        actions={<BlockSkeleton className="h-8 w-24" />}
        className="items-start border-b border-border"
        summary={<TextSkeleton length="short" size="meta" />}
        title="Code"
      />
      <RepositoryLatestActivityPending />
      <FileWorkbench
        className="lg:min-h-[calc(100dvh-var(--app-chrome))]"
        navigationOpen={navigationOpen}
        onNavigationOpenChange={setNavigationOpen}
        selectedPath={null}
      >
        <div className="min-w-0 px-2 py-3">
          <PendingSurface className="min-h-[220px]" delay label="Loading repository files">
            <FileNavigatorSkeleton />
          </PendingSurface>
        </div>
        <div>
          <SourceTabStripSkeleton />
          <PendingSurface className="min-h-[220px]" delay label="Loading repository introduction">
            <SourceCodeSkeleton />
          </PendingSurface>
        </div>
      </FileWorkbench>
    </WorkbenchPane>
  )
}
