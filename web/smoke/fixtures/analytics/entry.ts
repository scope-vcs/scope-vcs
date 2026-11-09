import { loadAnalyticsBootstrap } from '../../../src/analytics/bootstrap'
import { applyAnalyticsIdentityTransition } from '../../../src/analytics/client-identity'
import { pageViewProperties } from '../../../src/analytics/privacy'

declare global {
  interface Window {
    analyticsDelivered: (count: number) => Promise<void>
  }
}

const bootstrap = await loadAnalyticsBootstrap(new AbortController().signal)
if (bootstrap.client) {
  const client = bootstrap.client
  const capturePageView = () => client.capture('$pageview', pageViewProperties(
    { name: 'request_changes', path: '/repository/request/changes' },
    { origin: location.origin, referrer: 'https://search.example/private?q=secret', search: '?path=private.rs' },
  ))
  const captureError = (errorOrigin: 'route' | 'window') => client.capture('frontend_error', {
    error_kind: 'type_error', error_origin: errorOrigin, route_name: 'request_changes',
    message: 'SECRET source text', repository: 'owner/private-repository',
  })

  capturePageView()
  await window.analyticsDelivered(1)
  applyAnalyticsIdentityTransition(client, 'scope_usr_one', bootstrap.eventContext)
  await window.analyticsDelivered(2)
  captureError('window')
  await window.analyticsDelivered(3)

  applyAnalyticsIdentityTransition(client, null, bootstrap.eventContext)
  capturePageView()
  await window.analyticsDelivered(4)
  applyAnalyticsIdentityTransition(client, 'scope_usr_one', bootstrap.eventContext)
  await window.analyticsDelivered(5)
  applyAnalyticsIdentityTransition(client, 'scope_usr_two', bootstrap.eventContext)
  capturePageView()
  captureError('route')

  client.capture('unexpected_event', { secret: 'SECRET source text' })
  await window.analyticsDelivered(7)
}
Object.assign(window, { analyticsReady: true, analyticsEnabled: Boolean(bootstrap.client) })
