import { createCachedResource } from '../lib/cached-resource'
import { AnalyticsClient } from './client'
import {
  analyticsEventContext,
  registerAnalyticsEventContext,
  type AnalyticsEventContext,
} from './client-identity'
import { parseAnalyticsRuntimeConfig } from './config'

type AnalyticsBootstrap =
  | { client: AnalyticsClient; eventContext: AnalyticsEventContext }
  | { client: null; eventContext: null }

export const analyticsBootstrapResource = createCachedResource<AnalyticsBootstrap>({
  maxEntries: 1,
})

export async function loadAnalyticsBootstrap(signal: AbortSignal) {
  const config = await fetchAnalyticsRuntimeConfig(signal)
  if (!config) return { client: null, eventContext: null }

  const client = new AnalyticsClient(config.token, window.location.origin)
  const eventContext = analyticsEventContext(config)
  registerAnalyticsEventContext(client, eventContext)
  return { client, eventContext }
}

export async function fetchAnalyticsRuntimeConfig(
  signal: AbortSignal,
  fetcher: typeof fetch = fetch,
) {
  const response = await fetcher('/e/config', {
    cache: 'no-store',
    credentials: 'omit',
    headers: { Accept: 'application/json' },
    referrerPolicy: 'no-referrer',
    signal,
  })
  if (!response.ok) throw new Error('Analytics configuration is unavailable.')

  const value: unknown = await response.json()
  const config = parseAnalyticsRuntimeConfig(value)
  if (value !== null && config === null) {
    throw new Error('Analytics configuration is invalid.')
  }
  return config
}
