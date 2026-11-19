import { WorkbenchBar, WorkbenchBody, WorkbenchPane } from '@/components/page-header'
import { PendingSurface } from '@/components/pending-surface'
import { Button } from '@/components/ui/button'
import { BlockSkeleton } from '@/components/ui/block-skeleton'
import { TextSkeleton } from '@/components/ui/text-skeleton'
import { HistoryFeedSkeleton } from '@/features/history/history-feed-state'
import { useParams } from '@tanstack/react-router'
import {
  AccessSection,
  CiSection,
  DangerZoneSection,
  REPOSITORY_DETAIL_FIELDS,
  RepositoryDetailsSection,
  VisibilityChangesSection,
} from './repo-settings-sections'
import { VIEW_ROW_CLASS, ViewsSection } from './repo-views-section'

const PENDING_VIEW_ROWS = ['public', 'private']

export function RepoSettingsPending() {
  const { owner } = useParams({ from: '/$owner/$repo' })
  return (
    <PendingSurface label="Loading repository settings">
      <WorkbenchPane>
        <WorkbenchBar title="Settings" />
        <WorkbenchBody>
          <RepositoryDetailsSection>
            <div className="max-w-xl space-y-4">
              {Object.values(REPOSITORY_DETAIL_FIELDS).map((label) => (
                <div key={label}>
                  <div className="mb-1.5 text-sm font-medium">{label}</div>
                  <BlockSkeleton className="h-10 w-full rounded-lg" />
                </div>
              ))}
              <Button disabled size="sm" type="button">Save details</Button>
            </div>
          </RepositoryDetailsSection>
          <ViewsSection>
            <ul className="divide-y divide-border text-sm">
              {PENDING_VIEW_ROWS.map((row) => (
                <li className={VIEW_ROW_CLASS} key={row}>
                  <TextSkeleton length="short" />
                  <TextSkeleton length="medium" size="meta" />
                </li>
              ))}
            </ul>
          </ViewsSection>
          <VisibilityChangesSection>
            <HistoryFeedSkeleton />
          </VisibilityChangesSection>
          <CiSection>
            <TextSkeleton length="long" />
          </CiSection>
          <DangerZoneSection />
          <AccessSection canInvite ownerHandle={owner} />
        </WorkbenchBody>
      </WorkbenchPane>
    </PendingSurface>
  )
}
