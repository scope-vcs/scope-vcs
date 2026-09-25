import { WorkbenchPane } from '@/components/page-header'
import { PendingSurface } from '@/components/pending-surface'
import { useParams } from '@tanstack/react-router'
import { UpdateNavigation } from './update-navigation'
import { UpdateDetailSkeleton } from './update-detail-skeleton'

export function UpdatePagePending() {
  const { owner, repo } = useParams({ from: '/$owner/$repo' })
  return (
    <PendingSurface label="Loading update">
      <WorkbenchPane>
        <UpdateNavigation newer={null} older={null} params={{ owner, repo }} search={{}} />
        <UpdateDetailSkeleton />
      </WorkbenchPane>
    </PendingSurface>
  )
}
