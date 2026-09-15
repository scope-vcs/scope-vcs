import { loadAnalyticsBootstrap } from '../../../src/analytics/bootstrap'
import { applyAnalyticsIdentityTransition } from '../../../src/analytics/client-identity'
import { pageViewProperties } from '../../../src/analytics/privacy'

const bootstrap = await loadAnalyticsBootstrap(new AbortController().signal)
if (bootstrap.client) {
  const client = bootstrap.client
  const capturePageView = () => client.capture('$pageview', pageViewProperties(
    { name: 'request_details', path: '/repository/request/details' },
    { origin: location.origin, referrer: 'https://search.example/private?q=secret', search: '?path=private.rs' },
  ))
  const captureError = (errorOrigin: 'route' | 'window') => client.capture('frontend_error', {
    error_kind: 'type_error', error_origin: errorOrigin, route_name: 'request_details',
    message: 'SECRET source text', repository: 'owner/private-repository',
  })

  capturePageView()
  applyAnalyticsIdentityTransition(client, 'scope_usr_one', bootstrap.eventContext)
  captureError('window')

  applyAnalyticsIdentityTransition(client, null, bootstrap.eventContext)
  capturePageView()
  applyAnalyticsIdentityTransition(client, 'scope_usr_one', bootstrap.eventContext)
  applyAnalyticsIdentityTransition(client, 'scope_usr_two', bootstrap.eventContext)
  capturePageView()
  captureError('route')

  client.capture('unexpected_event', { secret: 'SECRET source text' })
}
Object.assign(window, { analyticsReady: true, analyticsEnabled: Boolean(bootstrap.client) })
