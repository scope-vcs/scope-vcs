import { WorkbenchBar, WorkbenchBody, WorkbenchPane } from '@/components/page-header'
import { PendingSurface } from '@/components/pending-surface'
import { Button } from '@/components/ui/button'
import { BlockSkeleton } from '@/components/ui/block-skeleton'
import { TextSkeleton } from '@/components/ui/text-skeleton'
import { useParams } from '@tanstack/react-router'
import {
  AccessSection,
  CiSection,
  DangerZoneSection,
  REPOSITORY_DETAIL_FIELDS,
  RepositoryDetailsSection,
} from './repo-settings-sections'

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
