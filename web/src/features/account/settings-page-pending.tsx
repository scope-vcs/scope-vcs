import { ApplicationPendingShell } from '@/components/pending-surface'
import { Button } from '@/components/ui/button'
import { Plus } from 'lucide-react'
import { ProfileSection } from './profile-section'
import {
  CliAccessSection,
  CliSessionListSkeleton,
  DeleteAccountRow,
  SettingsLayout,
} from './settings-sections'
import { ThemeSection } from './theme-section'

export function SettingsPagePending() {
  return (
    <ApplicationPendingShell contextLabel="Settings" label="Loading settings">
      <div className="py-8 lg:py-10">
        <SettingsLayout>
          <ProfileSection />
          <ThemeSection />
          <CliAccessSection
            action={(
              <Button disabled size="sm" type="button" variant="secondary">
                <Plus />
                <span>New login command</span>
              </Button>
            )}
          >
            <CliSessionListSkeleton />
          </CliAccessSection>
          <DeleteAccountRow />
        </SettingsLayout>
      </div>
    </ApplicationPendingShell>
  )
}
