import { useAuth } from '@clerk/tanstack-react-start'
import { useRouterState } from '@tanstack/react-router'
import type { PostHog } from 'posthog-js'
import { useEffect, useRef } from 'react'
import { useAccountSession } from '@/features/account/use-account-session'
import { useCachedResource, useRetryOnReconnect } from '@/lib/use-cached-resource'
import { useHydrated } from '@/lib/use-hydrated'
import {
  analyticsBootstrapResource,
  loadAnalyticsBootstrap,
} from './bootstrap'
import {
  applyAnalyticsIdentityTransition,
  type AnalyticsEventContext,
} from './client-identity'
import { installBrowserDiagnostics } from './diagnostics'
import {
  expectedIdentityKey,
  resolveAnalyticsIdentity,
} from './identity'
import { pageViewProperties } from './privacy'
import { analyticsRouteForId } from './routes'

const bootstrapIdentity = 'browser'

export function AnalyticsRoot() {
  const hydrated = useHydrated()
  const bootstrap = useCachedResource({
    enabled: hydrated,
    fallbackError: 'Analytics is unavailable.',
    identity: hydrated ? bootstrapIdentity : null,
    load: loadAnalyticsBootstrap,
    resource: analyticsBootstrapResource,
  })

  useRetryOnReconnect(bootstrap)

  return bootstrap.status === 'loaded' && bootstrap.value.client
    ? <AnalyticsRuntime
        client={bootstrap.value.client}
        eventContext={bootstrap.value.eventContext}
      />
    : null
}

function AnalyticsRuntime({
  client,
  eventContext,
}: {
  client: PostHog
  eventContext: AnalyticsEventContext
}) {
  const { isLoaded, isSignedIn, userId } = useAuth()
  const viewer = {
    clerkUserId: userId ?? null,
    isLoaded,
    isSignedIn: Boolean(isSignedIn),
  }
  const session = useAccountSession(
    isLoaded && isSignedIn && userId ? userId : null,
  )
  const identity = resolveAnalyticsIdentity({
    ...viewer,
    scopeUserId: session.value?.account?.user?.id ?? null,
    sessionResolved: session.status === 'loaded',
  })
  const routeId = useRouterState({
    select: (state) => state.matches.at(-1)?.routeId,
  })
  const pathname = useRouterState({
    select: (state) => state.location.pathname,
  })
  const routeName = analyticsRouteForId(routeId)?.name ?? null
  const capturedPage = useRef<string | null>(null)
  const documentRouteName = useRef(routeName)
  const appliedKey = useRef<string | null>(null)
  const expectedKey = useRef<string | null>(null)
  const currentPage = useRef({ pathname, routeId })
  const diagnostics = useRef<ReturnType<typeof installBrowserDiagnostics> | null>(null)

  currentPage.current = { pathname, routeId }
  expectedKey.current = expectedIdentityKey(viewer)

  const resolvedIdentityKey = identity?.identityKey ?? null
  const resolvedScopeUserId = identity?.scopeUserId ?? null

  // The resource resolves the identity; this only applies it and releases the
  // events that had to wait for an attributable viewer.
  useEffect(() => {
    if (resolvedIdentityKey === null) return

    capturedPage.current = null
    safely(() => applyAnalyticsIdentityTransition(
      client,
      resolvedScopeUserId,
      eventContext,
    ))
    appliedKey.current = resolvedIdentityKey
    captureCurrentPage(client, currentPage.current, capturedPage)
    diagnostics.current?.flushErrors()
    diagnostics.current?.flushVitals()
  }, [client, eventContext, resolvedIdentityKey, resolvedScopeUserId])

  useEffect(() => {
    if (!isLoaded || appliedKey.current !== expectedKey.current) return
    captureCurrentPage(client, { pathname, routeId }, capturedPage)
  }, [client, isLoaded, pathname, routeId])

  useEffect(() => {
    const installed = installBrowserDiagnostics({
      capture: (event, properties) => {
        if (
          expectedKey.current === null
          || appliedKey.current !== expectedKey.current
        ) {
          return false
        }
        safely(() => client.capture(event, properties))
        return true
      },
      routeName: documentRouteName.current,
    })
    diagnostics.current = installed
    return () => {
      diagnostics.current = null
      installed.dispose()
    }
  }, [client])

  useEffect(() => {
    diagnostics.current?.setRoute(routeName)
  }, [routeName])

  return null
}

function captureCurrentPage(
  client: PostHog,
  page: { pathname: string; routeId: string | undefined },
  capturedPage: { current: string | null },
) {
  const route = analyticsRouteForId(page.routeId)
  if (!route) return

  const pageKey = `${page.routeId}:${page.pathname}`
  if (capturedPage.current === pageKey) return
  capturedPage.current = pageKey

  safely(() => client.capture('$pageview', pageViewProperties(route, {
    origin: window.location.origin,
    referrer: document.referrer,
    search: window.location.search,
  })))
}

function safely(action: () => void) {
  try {
    action()
  } catch {
    // Analytics is best effort and cannot affect application behavior.
  }
}
