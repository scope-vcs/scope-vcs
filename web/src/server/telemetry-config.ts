type RuntimeEnvironment = Readonly<Record<string, string | undefined>>

const DEFAULT_POSTHOG_HOST = 'https://us.i.posthog.com'
const OTLP_REDIRECT_VARIABLE =
  /^OTEL_EXPORTER_OTLP_(?:(?:TRACES|LOGS)_)?(?:ENDPOINT|HEADERS)$/

export type TelemetryDisabledReason =
  | 'unconfigured'
  | 'invalid-host'
  | 'foreign-otlp-variables'

export type TelemetryConfig =
  | {
    enabled: true
    headers: { Authorization: string }
    logsUrl: string
    resourceAttributes: Record<string, string>
    tracesUrl: string
  }
  | { enabled: false; reason: TelemetryDisabledReason }

export function getTelemetryConfig(
  runtime: RuntimeEnvironment = process.env,
): TelemetryConfig {
  const token = trimmed(runtime.POSTHOG_PROJECT_TOKEN)
  if (!token) return disabled('unconfigured')
  if (Object.keys(runtime).some((name) => OTLP_REDIRECT_VARIABLE.test(name) && trimmed(runtime[name]))) {
    return disabled('foreign-otlp-variables')
  }
  const host = canonicalHost(runtime.POSTHOG_HOST ?? DEFAULT_POSTHOG_HOST)
  if (!host) return disabled('invalid-host')

  return {
    enabled: true,
    headers: { Authorization: `Bearer ${token}` },
    logsUrl: `${host}/i/v1/logs`,
    resourceAttributes: resourceAttributes(runtime),
    tracesUrl: `${host}/i/v1/traces`,
  }
}

export function telemetryDisabledMessage(reason: TelemetryDisabledReason) {
  switch (reason) {
    case 'unconfigured':
      return null
    case 'invalid-host':
      return 'Telemetry export is disabled because POSTHOG_HOST is not an http(s) origin.'
    case 'foreign-otlp-variables':
      return 'Telemetry export is disabled because Railway-injected OTEL_EXPORTER_OTLP endpoint or header variables are present and would redirect data or leak credentials.'
  }
}

function disabled(reason: TelemetryDisabledReason): TelemetryConfig {
  return { enabled: false, reason }
}

function resourceAttributes(runtime: RuntimeEnvironment) {
  const attributes: Record<string, string> = { 'service.name': 'web' }
  const release = trimmed(runtime.SCOPE_ANALYTICS_RELEASE)
  if (release) attributes['service.version'] = release
  const environment = trimmed(runtime.RAILWAY_ENVIRONMENT_NAME)
  if (environment) attributes['deployment.environment.name'] = environment
  return attributes
}

function canonicalHost(value: string) {
  const normalized = trimmed(value)
  if (!normalized) return null

  try {
    const url = new URL(normalized)
    if (
      (url.protocol !== 'https:' && url.protocol !== 'http:') ||
      url.username ||
      url.password ||
      url.pathname !== '/' ||
      url.search ||
      url.hash
    ) {
      return null
    }
    return url.origin
  } catch {
    return null
  }
}

function trimmed(value: string | undefined) {
  return value?.trim() || null
}
