type RuntimeEnvironment = Readonly<Record<string, string | undefined>>

const DEFAULT_POSTHOG_HOST = 'https://us.i.posthog.com'
const OTLP_REDIRECT_VARIABLE =
  /^OTEL_EXPORTER_OTLP_(?:(?:TRACES|LOGS)_)?(?:ENDPOINT|HEADERS)$/

export const INJECTED_OTLP_VARIABLES_MESSAGE =
  'PostHog telemetry export disabled because Railway still injects OTEL_EXPORTER_OTLP endpoint or header variables; disable Railway tracing for this service'

export type TelemetryConfig =
  | {
    enabled: true
    headers: { Authorization: string }
    logsUrl: string
    resourceAttributes: Record<string, string>
    tracesUrl: string
  }
  | { enabled: false; reason: 'unconfigured' | 'injected-otlp-variables' }

export function getTelemetryConfig(
  runtime: RuntimeEnvironment = process.env,
): TelemetryConfig {
  const token = trimmed(runtime.POSTHOG_PROJECT_TOKEN)
  if (!token) return { enabled: false, reason: 'unconfigured' }
  if (Object.keys(runtime).some((name) => OTLP_REDIRECT_VARIABLE.test(name) && trimmed(runtime[name]))) {
    return { enabled: false, reason: 'injected-otlp-variables' }
  }
  const host = (trimmed(runtime.POSTHOG_HOST) ?? DEFAULT_POSTHOG_HOST).replace(/\/+$/, '')

  return {
    enabled: true,
    headers: { Authorization: `Bearer ${token}` },
    logsUrl: `${host}/i/v1/logs`,
    resourceAttributes: resourceAttributes(runtime),
    tracesUrl: `${host}/i/v1/traces`,
  }
}

function resourceAttributes(runtime: RuntimeEnvironment) {
  const attributes: Record<string, string> = { 'service.name': 'web' }
  const release = trimmed(runtime.SCOPE_ANALYTICS_RELEASE)
  if (release) attributes['service.version'] = release
  const environment = trimmed(runtime.RAILWAY_ENVIRONMENT_NAME)
  if (environment) attributes['deployment.environment.name'] = environment
  return attributes
}

function trimmed(value: string | undefined) {
  return value?.trim() || null
}
