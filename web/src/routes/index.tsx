import { loadCliInstallStateForRequest } from '@/api/cli-install'
import { loadAuthenticatedAccountForRequest } from '@/api/profile'
import { ApplicationPendingShell } from '@/components/pending-surface'
import { MarketingLandingPage } from '@/features/marketing/marketing-landing-page'
import { createFileRoute, redirect } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'

const loadIndex = createServerFn({ method: 'GET' }).handler(async () => {
  const { auth } = await import('@clerk/tanstack-react-start/server')
  const { isAuthenticated } = await auth()

  if (!isAuthenticated) {
    return loadCliInstallStateForRequest()
  }

  const account = await loadAuthenticatedAccountForRequest()
  const handle = account.user?.handle
  if (!handle) {
    throw new Error('Signed-in account is missing a Scope handle.')
  }
  throw redirect({ params: { owner: handle }, to: '/$owner' })
})

export const Route = createFileRoute('/')({
  loader: () => loadIndex(),
  pendingComponent: IndexPending,
  component: IndexRoute,
})

function IndexPending() {
  return <ApplicationPendingShell label="Loading Scope" />
}

function IndexRoute() {
  const state = Route.useLoaderData()
  return (
    <MarketingLandingPage
      cliInstallCommands={state.cliInstallCommands}
      initialCliPlatform={state.initialCliPlatform}
    />
  )
}
