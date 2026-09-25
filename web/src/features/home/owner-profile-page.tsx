import type { ProfileState } from '@/api/types'
import { ApplicationTopbar } from '@/components/application-topbar'
import { AppShell } from '@/components/app-shell'
import { PageContent, PageHeader } from '@/components/page-header'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { useHomeFlash } from '@/lib/home-flash'
import { CheckCircle2 } from 'lucide-react'
import { FirstRepositoryWalkthrough } from './first-repository-walkthrough'
import { OwnerProfileTopbarActions } from './owner-profile-topbar-actions'
import { RepoList } from './repo-list'

export function OwnerProfilePage({ state }: { state: ProfileState }) {
  const flash = useHomeFlash()
  const { account, profile } = state
  const isOwner = account.user?.handle === profile.handle

  return (
    <AppShell
      header={() => (
        <ApplicationTopbar>
          <OwnerProfileTopbarActions handle={profile.handle} signedIn={Boolean(account.user)} />
        </ApplicationTopbar>
      )}
    >
      <PageContent>
        <PageHeader title={`@${profile.handle}`} />

        {flash && (
          <Alert className="mt-6">
            <CheckCircle2 className="size-4" />
            <AlertTitle>Success</AlertTitle>
            <AlertDescription>{flash}</AlertDescription>
          </Alert>
        )}

        {isOwner && profile.repositories.length === 0 ? (
          <FirstRepositoryWalkthrough
            cliInstallCommands={state.cliInstallCommands}
            initialCliPlatform={state.initialCliPlatform}
          />
        ) : (
          <RepoList isOwner={isOwner} repositories={profile.repositories} />
        )}
      </PageContent>
    </AppShell>
  )
}
