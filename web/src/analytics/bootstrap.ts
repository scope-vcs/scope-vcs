import { createCachedResource } from '../lib/cached-resource'
import type { PostHog, PostHogConfig } from 'posthog-js'
import {
  analyticsEventContext,
  registerAnalyticsEventContext,
  type AnalyticsEventContext,
} from './client-identity'
import { parseAnalyticsRuntimeConfig } from './config'
import { createPrivacyBoundary } from './privacy'

type AnalyticsBootstrap =
  | { client: PostHog; eventContext: AnalyticsEventContext }
  | { client: null; eventContext: null }

export const analyticsBootstrapResource = createCachedResource<AnalyticsBootstrap>({
  maxEntries: 1,
})

export async function loadAnalyticsBootstrap(signal: AbortSignal) {
  const config = await fetchAnalyticsRuntimeConfig(signal)
  if (!config) return { client: null, eventContext: null }

  const { default: posthog } = await import('posthog-js')
  const client = posthog.init(
    config.token,
    analyticsClientOptions(window.location.origin),
  )
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

export function analyticsClientOptions(
  origin: string,
): Partial<PostHogConfig> {
  return {
    advanced_disable_feature_flags: true,
    advanced_disable_feature_flags_on_first_load: true,
    advanced_disable_flags: true,
    api_host: '/e',
    autocapture: false,
    before_send: createPrivacyBoundary(origin),
    capture_dead_clicks: false,
    capture_exceptions: false,
    capture_heatmaps: false,
    capture_pageleave: false,
    capture_pageview: false,
    capture_performance: false,
    cross_subdomain_cookie: false,
    disable_capture_url_hashes: true,
    disable_conversations: true,
    disable_external_dependency_loading: true,
    disable_product_tours: true,
    disable_scroll_properties: true,
    disable_session_recording: true,
    disable_surveys: true,
    disable_surveys_automatic_display: true,
    disable_web_experiments: true,
    ip: false,
    person_profiles: 'identified_only',
    persistence: 'localStorage',
    rageclick: false,
    respect_dnt: true,
    save_campaign_params: false,
    save_referrer: false,
    ui_host: 'https://us.posthog.com',
  }
}
