import { WorkbenchBar, WorkbenchPane } from '@/components/page-header'
import { BlockSkeleton, TextSkeleton } from '@/components/ui/skeleton'
import { useWorkspaceTabs } from '@/components/use-workspace-tabs'
import type { CachedResource } from '@/lib/use-cached-resource'
import { useParams } from '@tanstack/react-router'
import { RepositoryCodeView } from './repository-code-view'
import { RepositoryLatestActivityPending } from './repository-latest-activity'

// The code view draws its own loading layout for resources it has not started,
// so the route's pending state is that same view with nothing requested yet.
export function RepositoryCodePending() {
  const params = useParams({ from: '/$owner/$repo' })
  const workspaceTabs = useWorkspaceTabs({ activeId: null })
  return (
    <WorkbenchPane>
      <WorkbenchBar
        actions={<BlockSkeleton className="h-8 w-24" />}
        className="items-start border-b border-border"
        summary={<TextSkeleton length="short" size="meta" />}
        title="Code"
      />
      <RepositoryLatestActivityPending />
      <RepositoryCodeView
        content={idleResource()}
        file={idleResource()}
        onSelectFilePath={() => {}}
        params={params}
        selectedPath={null}
        workspaceTabs={workspaceTabs}
      />
    </WorkbenchPane>
  )
}

function idleResource<T extends object>(): CachedResource<T> {
  return { error: null, identity: null, refreshing: false, retry: () => {}, status: 'idle', value: null }
}
