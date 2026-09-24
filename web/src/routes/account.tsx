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
import { SectionRows } from '@/components/section-rows'
import { Button } from '@/components/ui/button'
import { AccountPageHeader } from '@/features/account/account-page-header'
import { AccountPagePending } from '@/features/account/account-page-pending'
import { CliLoginSection, CliSessionsSection } from '@/features/account/account-sections'
import { CliSessionList } from '@/features/account/cli-session-list'
import { UserButton } from '@clerk/tanstack-react-start'
import { AbsoluteTimestamp } from '@/components/timestamp'
import { createFileRoute, redirect } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import { LoaderCircle, Plus } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'
import type { CliExchangeGrantResponse } from '@/api/types.generated'
import { usePendingActions } from '@/lib/use-pending-actions'

const requireAccountAuth = createServerFn({ method: 'GET' }).handler(async () => {
  const { auth } = await import('@clerk/tanstack-react-start/server')
  const { isAuthenticated } = await auth()
  if (!isAuthenticated) {
    throw redirect({ params: { _splat: '' }, to: '/sign-in/$' })
  }
})

const loadCliSessions = createServerFn({ method: 'GET' }).handler(
  listCliSessionsForRequest,
)

const createCliExchangeGrant = createServerFn({ method: 'POST' }).handler(
  createCliExchangeGrantForRequest,
)

const revokeCliSession = createServerFn({ method: 'POST' })
  .validator(parseRevokeCliSessionInput)
  .handler(({ data }) => revokeCliSessionForRequest(data))

export const Route = createFileRoute('/account')({
  beforeLoad: () => requireAccountAuth(),
  loader: () => loadCliSessions(),
  pendingComponent: AccountPagePending,
  component: AccountRoute,
})

function AccountRoute() {
  const loaded = Route.useLoaderData()
  const [sessions, setSessions] = useState(() => loaded.sessions)
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
        setSessions((current) => current.filter((session) => session.id !== sessionId))
        toast.success('CLI session revoked')
      } catch (error) {
        setError(error instanceof Error ? error.message : 'Could not revoke CLI session')
      }
    })
  }

  return (
    <AppShell
      header={() => (
        <ApplicationTopbar contextLabel="Account">
          <UserButton />
        </ApplicationTopbar>
      )}
    >
      <PageContent>
        <AccountPageHeader />

        {error && (
          <PageErrorAlert title="CLI session update failed">
            {error}
          </PageErrorAlert>
        )}

        <SectionRows>
          <CliLoginSection>
            <div className="space-y-3">
              <Button
                disabled={pending.has('grant')}
                onClick={() => void createGrant()}
                size="sm"
                type="button"
              >
                {pending.has('grant') ? (
                  <LoaderCircle className="size-3.5 animate-spin" />
                ) : (
                  <Plus className="size-3.5" />
                )}
                <span>{grant ? 'Create another' : 'Create command'}</span>
              </Button>
              {grant && (
                <div className="space-y-2">
                  <CopyableCodeBlock value={`scope login --exchange ${grant.exchange_token}`} />
                  <p className="text-xs leading-4 text-muted-foreground">
                    <AbsoluteTimestamp prefix="Expires " value={grant.expires_at_unix} />.
                  </p>
                </div>
              )}
            </div>
          </CliLoginSection>

          <CliSessionsSection>
            <CliSessionList
              pending={pending}
              revokeSession={(sessionId) => void revokeSession(sessionId)}
              sessions={sessions}
            />
          </CliSessionsSection>
        </SectionRows>
      </PageContent>
    </AppShell>
  )
}
