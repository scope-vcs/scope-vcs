import { loadAnalyticsBootstrap } from '../../../src/analytics/bootstrap'
import { pageViewProperties } from '../../../src/analytics/privacy'

const bootstrap = await loadAnalyticsBootstrap(new AbortController().signal)
if (bootstrap.client) {
  const client = bootstrap.client
  client.capture('$pageview', pageViewProperties(
    { name: 'request_details', path: '/repository/request/details' },
    { origin: location.origin, referrer: 'https://search.example/private?q=secret', search: '?path=private.rs' },
  ))
  client.identify('scope_usr_browser_fixture')
  client.capture('frontend_error', {
    error_kind: 'type_error', error_origin: 'window', route_name: 'request_details',
    message: 'SECRET source text', repository: 'owner/private-repository',
  })
  client.capture('unexpected_event', { secret: 'SECRET source text' })
}
Object.assign(window, { analyticsReady: true, analyticsEnabled: Boolean(bootstrap.client) })
