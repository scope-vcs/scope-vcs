import { FileWorkbench } from '@/components/file-workbench'
import { WorkbenchBar, WorkbenchPane } from '@/components/page-header'
import { PendingSurface } from '@/components/pending-surface'
import { BlockSkeleton } from '@/components/ui/block-skeleton'
import { useState } from 'react'
import {
  FileNavigatorSkeleton,
  SourceCodeSkeleton,
  SourceTabStripSkeleton,
} from './repository-code-skeletons'
import { useRepoLayout } from './repo-layout-context'
import { hasRepositoryAbout, RepositoryAbout } from './repository-about'
import { RepositoryLatestActivityPending } from './repository-latest-activity'

export function RepositoryCodePending() {
  const [navigationOpen, setNavigationOpen] = useState(false)
  const { repo } = useRepoLayout()
  return (
    <WorkbenchPane>
      <WorkbenchBar
        actions={<BlockSkeleton className="h-8 w-24" />}
        summary={hasRepositoryAbout(repo) ? <RepositoryAbout repo={repo} /> : undefined}
        title="Code"
      />
      {repo.lifecycle_state === 'Ready' && <RepositoryLatestActivityPending />}
      <FileWorkbench
        className="border-t border-border lg:min-h-[calc(100dvh-var(--app-chrome))]"
        navigationOpen={navigationOpen}
        onNavigationOpenChange={setNavigationOpen}
        selectedPath={null}
      >
        <div className="min-w-0 py-3">
          <PendingSurface className="min-h-[220px]" label="Loading repository files">
            <FileNavigatorSkeleton />
          </PendingSurface>
        </div>
        <div>
          <SourceTabStripSkeleton />
          <PendingSurface className="min-h-[220px]" label="Loading repository introduction">
            <SourceCodeSkeleton />
          </PendingSurface>
        </div>
      </FileWorkbench>
    </WorkbenchPane>
  )
}
