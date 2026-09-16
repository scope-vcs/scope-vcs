import type { CaptureResult, Properties, Property } from 'posthog-js'
import {
  analyticsRouteForName,
  type AnalyticsRoute,
} from './routes'

const transportPropertyNames = [
  '$anon_distinct_id',
  '$device_id',
  '$is_identified',
  '$lib',
  '$lib_version',
  '$process_person_profile',
  '$session_id',
  '$user_id',
  '$window_id',
  'distinct_id',
  'token',
] as const

const campaignPropertyNames = [
  'utm_campaign',
  'utm_medium',
  'utm_source',
] as const

const campaignValuePattern = /^[a-zA-Z0-9._-]{1,80}$/
const releaseValuePattern = /^[a-zA-Z0-9._-]{1,120}$/
const errorKinds = new Set([
  'abort_error',
  'aggregate_error',
  'dom_error',
  'eval_error',
  'range_error',
  'reference_error',
  'syntax_error',
  'type_error',
  'unknown_error',
  'uri_error',
])
const errorOrigins = new Set(['hydration', 'promise', 'route', 'window'])
const webVitalMetrics = new Set(['CLS', 'INP', 'LCP'])

type PageViewContext = {
  origin: string
  referrer: string
  search: string
}

export function pageViewProperties(
  route: AnalyticsRoute,
  context: PageViewContext,
): Properties {
  const properties: Properties = {
    $current_url: `${context.origin}${route.path}`,
    $host: new URL(context.origin).host,
    $pathname: route.path,
    route_name: route.name,
  }
  const referrer = externalReferrer(context.referrer, context.origin)
  if (referrer) {
    properties.$referrer = referrer.origin
    properties.$referring_domain = referrer.host
  }

  const search = new URLSearchParams(context.search)
  for (const propertyName of campaignPropertyNames) {
    const value = search.get(propertyName)
    if (value && campaignValuePattern.test(value)) {
      properties[propertyName] = value
    }
  }

  return properties
}

export function createPrivacyBoundary(origin: string) {
  const siteOrigin = new URL(origin).origin
  return (capture: CaptureResult | null) => sanitizeCapture(capture, siteOrigin)
}

export function sanitizeCapture(
  capture: CaptureResult | null,
  siteOrigin: string,
): CaptureResult | null {
  if (!capture) return null

  if (capture.event === '$identify') {
    return withoutPersonMutations(
      capture,
      transportProperties(capture.properties),
    )
  }

  if (capture.event === 'frontend_error') {
    return sanitizeFrontendError(capture)
  }

  if (capture.event === 'web_vital') {
    return sanitizeWebVital(capture)
  }

  if (capture.event !== '$pageview') return null

  const routeName = capture.properties.route_name
  if (typeof routeName !== 'string') return null

  const route = analyticsRouteForName(routeName)
  if (!route) return null

  const origin = new URL(siteOrigin).origin
  const properties: Properties = {
    ...transportProperties(capture.properties),
    $current_url: `${origin}${route.path}`,
    $host: new URL(origin).host,
    $pathname: route.path,
    route_name: route.name,
  }

  copyCampaignProperties(capture.properties, properties)
  copyExternalReferrer(capture.properties, properties, origin)

  return withoutPersonMutations(capture, properties)
}

function withoutPersonMutations(
  capture: CaptureResult,
  properties: Properties,
) {
  const sanitized = { ...capture, properties }
  delete sanitized.$set
  delete sanitized.$set_once
  delete sanitized.$unset
  return sanitized
}

function transportProperties(properties: Properties) {
  const allowed: Properties = { $geoip_disable: true }
  for (const propertyName of transportPropertyNames) {
    const value = properties[propertyName]
    if (isPostHogProperty(value)) allowed[propertyName] = value
  }
  if (properties.environment === 'production' || properties.environment === 'test') {
    allowed.environment = properties.environment
  }
  if (properties.release === null) {
    allowed.release = null
  } else if (
    typeof properties.release === 'string'
    && releaseValuePattern.test(properties.release)
  ) {
    allowed.release = properties.release
  }
  if (properties.source === 'browser') allowed.source = 'browser'
  return allowed
}

function sanitizeFrontendError(capture: CaptureResult) {
  const { error_kind: errorKind, error_origin: errorOrigin } = capture.properties
  const route = safeRoute(capture.properties.route_name)
  if (
    !route
    || typeof errorKind !== 'string'
    || !errorKinds.has(errorKind)
    || typeof errorOrigin !== 'string'
    || !errorOrigins.has(errorOrigin)
  ) {
    return null
  }

  return withoutPersonMutations(capture, {
    ...transportProperties(capture.properties),
    error_kind: errorKind,
    error_origin: errorOrigin,
    route_name: route.name,
  })
}

function sanitizeWebVital(capture: CaptureResult) {
  const { metric, value } = capture.properties
  const route = safeRoute(capture.properties.route_name)
  if (
    !route
    || typeof metric !== 'string'
    || !webVitalMetrics.has(metric)
    || typeof value !== 'number'
    || !Number.isFinite(value)
    || value < 0
    || value > (metric === 'CLS' ? 100 : 600_000)
  ) {
    return null
  }

  return withoutPersonMutations(capture, {
    ...transportProperties(capture.properties),
    metric,
    route_name: route.name,
    value,
  })
}

function safeRoute(routeName: Property | undefined) {
  return typeof routeName === 'string'
    ? analyticsRouteForName(routeName)
    : null
}

function copyCampaignProperties(source: Properties, target: Properties) {
  for (const propertyName of campaignPropertyNames) {
    const value = source[propertyName]
    if (typeof value === 'string' && campaignValuePattern.test(value)) {
      target[propertyName] = value
    }
  }
}

function copyExternalReferrer(
  source: Properties,
  target: Properties,
  siteOrigin: string,
) {
  const value = source.$referrer
  if (typeof value !== 'string') return

  const referrer = externalReferrer(value, siteOrigin)
  if (!referrer) return

  target.$referrer = referrer.origin
  target.$referring_domain = referrer.host
}

function externalReferrer(referrer: string, siteOrigin: string) {
  if (!referrer) return null

  try {
    const parsed = new URL(referrer)
    return parsed.origin === new URL(siteOrigin).origin ? null : parsed
  } catch {
    return null
  }
}

function isPostHogProperty(value: Property | undefined): value is Property {
  return value !== undefined
}
