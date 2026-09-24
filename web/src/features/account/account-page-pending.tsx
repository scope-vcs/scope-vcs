import { ApplicationPendingShell } from '@/components/pending-surface'
import { SectionRows } from '@/components/section-rows'
import { Button } from '@/components/ui/button'
import { UserButton } from '@clerk/tanstack-react-start'
import { Plus } from 'lucide-react'
import { AccountPageHeader } from './account-page-header'
import { CliLoginSection, CliSessionListSkeleton, CliSessionsSection } from './account-sections'

// Only the session list is data; the rest of the page is drawn as loaded.
export function AccountPagePending() {
  return (
    <ApplicationPendingShell actions={<UserButton />} contextLabel="Account" label="Loading account">
      <div className="py-8 lg:py-10">
        <AccountPageHeader />
        <SectionRows>
          <CliLoginSection>
            <div className="space-y-3">
              <Button disabled size="sm" type="button">
                <Plus className="size-3.5" />
                <span>Create command</span>
              </Button>
            </div>
          </CliLoginSection>
          <CliSessionsSection>
            <CliSessionListSkeleton />
          </CliSessionsSection>
        </SectionRows>
      </div>
    </ApplicationPendingShell>
  )
}
