import { deleteAccountForRequest } from '@/api/account'
import {
  createCliExchangeGrantForRequest,
  listCliSessionsForRequest,
  revokeCliSessionForRequest,
} from '@/api/cli-login'
import { parseRevokeCliSessionInput } from '@/api/cli-login-input'
import { ApplicationTopbar } from '@/components/application-topbar'
import { AppShell } from '@/components/app-shell'
import { CopyableCodeBlock } from '@/components/copyable-code-block'
import { PageContent } from '@/components/page-header'
import { PageErrorAlert } from '@/components/page-error-alert'
import { Button } from '@/components/ui/button'
import { ProfileSection } from '@/features/account/profile-section'
import { CliAccessSection, SettingsLayout } from '@/features/account/settings-sections'
import { SettingsPagePending } from '@/features/account/settings-page-pending'
import { ThemeSection } from '@/features/account/theme-section'
import { CliSessionList } from '@/features/account/cli-session-list'
import {
  loadCliSessionsHandoff,
  retainRevokedCliSession,
  useCliSessionsResource,
} from '@/features/account/cli-sessions-resource'
import { DeleteAccountSection } from '@/features/account/delete-account-section'
import { useAuth } from '@clerk/tanstack-react-start'
import { AbsoluteTimestamp } from '@/components/timestamp'
import { createFileRoute, redirect } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import { LoaderCircle, Plus } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'
import type { CliExchangeGrantResponse } from '@/api/types.generated'
import { usePendingActions } from '@/lib/use-pending-actions'

const requireSettingsAuth = createServerFn({ method: 'GET' }).handler(async () => {
  const { auth } = await import('@clerk/tanstack-react-start/server')
  const { isAuthenticated } = await auth()
  if (!isAuthenticated) {
    throw redirect({ params: { _splat: '' }, to: '/sign-in/$' })
  }
})

const loadCliSessions = createServerFn({ method: 'GET' }).handler(async () => {
  const { auth } = await import('@clerk/tanstack-react-start/server')
  const [{ userId }, sessions] = await Promise.all([auth(), listCliSessionsForRequest()])
  return { viewerId: userId, sessions }
})

const loadSessionsForResource = async () => (await loadCliSessions()).sessions

const createCliExchangeGrant = createServerFn({ method: 'POST' }).handler(
  createCliExchangeGrantForRequest,
)

const deleteAccount = createServerFn({ method: 'POST' }).handler(deleteAccountForRequest)

const revokeCliSession = createServerFn({ method: 'POST' })
  .validator(parseRevokeCliSessionInput)
  .handler(({ data }) => revokeCliSessionForRequest(data))

export const Route = createFileRoute('/settings')({
  beforeLoad: () => requireSettingsAuth(),
  loader: () => loadCliSessionsHandoff(loadCliSessions),
  pendingComponent: SettingsPagePending,
  component: SettingsRoute,
})

function SettingsRoute() {
  const loaded = Route.useLoaderData()
  const { userId } = useAuth()
  const { sessions } = useCliSessionsResource({
    viewerId: userId ?? null,
    handoff: loaded,
    load: loadSessionsForResource,
  })
  const [grant, setGrant] = useState<CliExchangeGrantResponse | null>(null)
  const { pending, run } = usePendingActions()
  const [error, setError] = useState<string | null>(null)

  async function createGrant() {
    await run('grant', async () => {
      setError(null)
      try {
        setGrant(await createCliExchangeGrant())
        toast.success('Login command created')
      } catch (error) {
        setError(error instanceof Error ? error.message : 'Could not create login command')
      }
    })
  }

  async function revokeSession(sessionId: string) {
    await run(sessionId, async () => {
      setError(null)
      try {
        await revokeCliSession({ data: { sessionId } })
        if (userId) retainRevokedCliSession(userId, sessionId)
        toast.success('CLI session revoked')
      } catch (error) {
        setError(error instanceof Error ? error.message : 'Could not revoke CLI session')
      }
    })
  }

  return (
    <AppShell header={() => <ApplicationTopbar contextLabel="Settings" />}>
      <PageContent>
        <SettingsLayout>
          <ProfileSection />
          <ThemeSection />
          <CliAccessSection
            action={(
              <Button
                disabled={pending.has('grant')}
                onClick={() => void createGrant()}
                size="sm"
                type="button"
                variant="secondary"
              >
                {pending.has('grant') ? <LoaderCircle className="animate-spin" /> : <Plus />}
                <span>{grant ? 'Create another' : 'New login command'}</span>
              </Button>
            )}
          >
            {error && (
              <PageErrorAlert className="mt-0 mb-4" title="CLI session update failed">
                {error}
              </PageErrorAlert>
            )}
            {grant && (
              <div className="mb-4 space-y-2">
                <CopyableCodeBlock value={`scope login --exchange ${grant.exchange_token}`} />
                <p className="text-xs leading-4 text-muted-foreground">
                  <AbsoluteTimestamp prefix="Expires " value={grant.expires_at_unix} />.
                </p>
              </div>
            )}
            <CliSessionList
              pending={pending}
              revokeSession={(sessionId) => void revokeSession(sessionId)}
              sessions={sessions}
            />
          </CliAccessSection>
          <DeleteAccountSection deleteAccount={() => deleteAccount()} />
        </SettingsLayout>
      </PageContent>
    </AppShell>
  )
}
