export type AnalyticsEnvironment = 'production' | 'test'

export type AnalyticsRuntimeConfig = {
  token: string
  environment: AnalyticsEnvironment
  release: string | null
}

export function parseAnalyticsRuntimeConfig(value: unknown): AnalyticsRuntimeConfig | null {
  if (value === null) return null
  if (!isRecord(value)) return null

  const { environment, release, token } = value
  if (
    (environment !== 'production' && environment !== 'test') ||
    typeof token !== 'string' ||
    token.length === 0 ||
    (release !== null && typeof release !== 'string')
  ) {
    return null
  }

  return { environment, release, token }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}
