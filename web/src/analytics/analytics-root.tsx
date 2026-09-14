import { useAuth } from '@clerk/tanstack-react-start'
import { useRouterState } from '@tanstack/react-router'
import type { PostHog } from 'posthog-js'
import { useEffect, useRef } from 'react'
import { loadAccountSessionForViewer } from '@/features/account/account-session-resource'
import { useCachedResource } from '@/lib/use-cached-resource'
import { useHydrated } from '@/lib/use-hydrated'
import { loadAccountSession } from '@/routes/-account-session-actions'
import {
  analyticsBootstrapResource,
  loadAnalyticsBootstrap,
} from './bootstrap'
import { installBrowserDiagnostics } from './diagnostics'
import {
  identifiedKey,
  identityTransition,
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

  useEffect(() => {
    if (bootstrap.status !== 'failed') return
    const retry = () => bootstrap.retry()
    window.addEventListener('focus', retry)
    window.addEventListener('online', retry)
    return () => {
      window.removeEventListener('focus', retry)
      window.removeEventListener('online', retry)
    }
  }, [bootstrap.retry, bootstrap.status])

  return bootstrap.status === 'loaded' && bootstrap.value.client
    ? <AnalyticsRuntime client={bootstrap.value.client} />
    : null
}

function AnalyticsRuntime({ client }: { client: PostHog }) {
  const { isLoaded, isSignedIn, userId } = useAuth()
  const routeId = useRouterState({
    select: (state) => state.matches.at(-1)?.routeId,
  })
  const pathname = useRouterState({
    select: (state) => state.location.pathname,
  })
  const capturedPage = useRef<string | null>(null)
  const identityKey = useRef<string | null>(null)
  const expectedIdentityKey = useRef<string | null>(null)
  const currentPage = useRef({ pathname, routeId })
  const diagnostics = useRef<ReturnType<typeof installBrowserDiagnostics> | null>(null)

  currentPage.current = { pathname, routeId }
  expectedIdentityKey.current = isLoaded
    ? isSignedIn && userId ? identifiedKey(userId) : 'anonymous'
    : null

  useEffect(() => {
    if (!isLoaded) return

    capturedPage.current = null
    if (!isSignedIn || !userId) {
      applyIdentityTransition(client, null)
      identityKey.current = 'anonymous'
      captureCurrentPage(client, currentPage.current, capturedPage)
      diagnostics.current?.flushErrors()
      diagnostics.current?.flushVitals()
      return
    }

    identityKey.current = null
    let active = true
    const resolveIdentity = async () => {
      try {
        const identity = await resolveAnalyticsIdentity(
          userId,
          () => loadAccountSessionForViewer(
            userId,
            (signal) => loadAccountSession({ signal }),
          ),
        )
        if (!active) return
        applyIdentityTransition(client, identity.scopeUserId)
        identityKey.current = identity.identityKey
        captureCurrentPage(client, currentPage.current, capturedPage)
        diagnostics.current?.flushErrors()
        diagnostics.current?.flushVitals()
      } catch {
        // A focus or online lifecycle event retries the retained resource.
      }
    }
    const retryUnresolvedIdentity = () => {
      if (active && identityKey.current === null) void resolveIdentity()
    }
    void resolveIdentity()
    window.addEventListener('focus', retryUnresolvedIdentity)
    window.addEventListener('online', retryUnresolvedIdentity)

    return () => {
      active = false
      window.removeEventListener('focus', retryUnresolvedIdentity)
      window.removeEventListener('online', retryUnresolvedIdentity)
    }
  }, [client, isLoaded, isSignedIn, userId])

  useEffect(() => {
    if (!isLoaded || identityKey.current !== expectedIdentityKey.current) return
    captureCurrentPage(client, { pathname, routeId }, capturedPage)
  }, [client, isLoaded, pathname, routeId])

  useEffect(() => {
    const installed = installBrowserDiagnostics({
      capture: (event, properties) => {
        if (
          expectedIdentityKey.current === null
          || identityKey.current !== expectedIdentityKey.current
        ) {
          return false
        }
        safely(() => client.capture(event, properties))
        return true
      },
      routeName: analyticsRouteForId(routeId)?.name ?? null,
    })
    diagnostics.current = installed
    return () => {
      diagnostics.current = null
      installed.dispose()
    }
  }, [client])

  useEffect(() => {
    diagnostics.current?.setRoute(analyticsRouteForId(routeId)?.name ?? null)
  }, [routeId])

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

function applyIdentityTransition(client: PostHog, scopeUserId: string | null) {
  safely(() => {
    const transition = identityTransition({
      currentDistinctId: client.get_distinct_id(),
      isSignedIn: Boolean(scopeUserId),
      persistedUserId: client.get_property('$user_id'),
      scopeUserId,
    })

    if (transition.kind === 'identify') {
      client.identify(transition.scopeUserId)
    } else if (transition.kind === 'reset_and_identify') {
      client.reset()
      client.identify(transition.scopeUserId)
    } else if (transition.kind === 'reset') {
      client.reset()
    }
  })
}

function safely(action: () => void) {
  try {
    action()
  } catch {
    // Analytics is best effort and cannot affect application behavior.
  }
}
