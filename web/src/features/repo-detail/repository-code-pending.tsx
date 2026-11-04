import { FileWorkbench } from '@/components/file-workbench'
import { WorkbenchBar, WorkbenchPane } from '@/components/page-header'
import { PendingSurface } from '@/components/pending-surface'
import { BlockSkeleton } from '@/components/ui/block-skeleton'
import { useLocation } from '@tanstack/react-router'
import { useState } from 'react'
import {
  FileNavigatorSkeleton,
  ReadmeSkeleton,
  SourceCodeSkeleton,
  SourceTabStripSkeleton,
} from './repository-code-skeletons'
import { useLoadedRepoLayout } from './repo-layout-context'
import { hasRepositoryAbout, RepositoryAbout } from './repository-about'
import { RepositoryLatestActivityPending } from './repository-latest-activity'

export function RepositoryCodePending() {
  const onCodePage = useLocation({
    select: ({ pathname }) => pathname.split('/').filter(Boolean)[2] === 'code',
  })
  return onCodePage ? <RepositoryBrowserPending /> : <RepositoryHomePending />
}

function RepositoryHomePending() {
  const repo = useLoadedRepoLayout()?.repo ?? null
  return (
    <WorkbenchPane>
      <WorkbenchBar
        actions={(
          <>
            <BlockSkeleton className="h-8 w-16" />
            <BlockSkeleton className="h-8 w-24" />
            <BlockSkeleton className="h-8 w-32" />
          </>
        )}
        summary={repo && hasRepositoryAbout(repo) ? <RepositoryAbout repo={repo} /> : undefined}
        title="Home"
        titleHidden
      />
      <section aria-label="Repository README" className="border-t border-border">
        <PendingSurface className="min-h-[220px]" label="Loading repository README">
          <ReadmeSkeleton />
        </PendingSurface>
      </section>
    </WorkbenchPane>
  )
}

function RepositoryBrowserPending() {
  const [navigationOpen, setNavigationOpen] = useState(false)
  const repo = useLoadedRepoLayout()?.repo ?? null
  return (
    <WorkbenchPane>
      <WorkbenchBar
        actions={<BlockSkeleton className="h-8 w-24" />}
        summary={<BlockSkeleton className="h-8 w-40" />}
        title="Code"
      />
      {repo?.lifecycle_state !== 'AwaitingFirstPush' && <RepositoryLatestActivityPending />}
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
