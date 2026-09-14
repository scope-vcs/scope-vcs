import type {
  AnalyticsEnvironment,
  AnalyticsRuntimeConfig,
} from '../analytics/config'

type RuntimeEnvironment = Readonly<Record<string, string | undefined>>

export function getAnalyticsRuntimeConfig(
  requestUrl: string | URL,
  runtime: RuntimeEnvironment = process.env,
): AnalyticsRuntimeConfig | null {
  const environment = analyticsEnvironment(runtime.SCOPE_ANALYTICS_ENVIRONMENT)
  const origin = canonicalOrigin(runtime.SCOPE_ANALYTICS_ORIGIN)
  const token = trimmed(runtime.POSTHOG_PROJECT_TOKEN)

  if (
    !environment ||
    !origin ||
    !token ||
    new URL(requestUrl).origin !== origin ||
    !deploymentMatches(environment, runtime.RAILWAY_ENVIRONMENT_NAME)
  ) {
    return null
  }

  return {
    environment,
    release: trimmed(runtime.SCOPE_ANALYTICS_RELEASE),
    token,
  }
}

function analyticsEnvironment(value: string | undefined): AnalyticsEnvironment | null {
  const normalized = trimmed(value)
  return normalized === 'production' || normalized === 'test' ? normalized : null
}

function canonicalOrigin(value: string | undefined) {
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

function deploymentMatches(
  environment: AnalyticsEnvironment,
  railwayEnvironment: string | undefined,
) {
  const railway = trimmed(railwayEnvironment)
  if (!railway) return true
  return environment === 'production' ? railway === 'production' : railway !== 'production'
}

function trimmed(value: string | undefined) {
  return value?.trim() || null
}
