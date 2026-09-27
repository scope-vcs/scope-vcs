import type { AnalyticsClient } from './client'
import type { AnalyticsRuntimeConfig } from './config'
import { identityTransition } from './identity'

export type AnalyticsEventContext = Readonly<{
  environment: AnalyticsRuntimeConfig['environment']
  release: AnalyticsRuntimeConfig['release']
  source: 'browser'
}>

type AnalyticsIdentityClient = Pick<
  AnalyticsClient,
  'get_distinct_id' | 'get_property' | 'identify' | 'register' | 'reset'
>

export function analyticsEventContext(
  config: AnalyticsRuntimeConfig,
): AnalyticsEventContext {
  return Object.freeze({
    environment: config.environment,
    release: config.release,
    source: 'browser' as const,
  })
}

export function registerAnalyticsEventContext(
  client: Pick<AnalyticsClient, 'register'>,
  context: AnalyticsEventContext,
) {
  client.register(context)
}

export function applyAnalyticsIdentityTransition(
  client: AnalyticsIdentityClient,
  scopeUserId: string | null,
  context: AnalyticsEventContext,
) {
  const transition = identityTransition({
    currentDistinctId: client.get_distinct_id(),
    isSignedIn: Boolean(scopeUserId),
    identifiedUserId: client.get_property('$user_id'),
    scopeUserId,
  })

  if (transition.kind === 'identify') {
    client.identify(transition.scopeUserId)
  } else if (transition.kind === 'reset_and_identify') {
    client.reset()
    registerAnalyticsEventContext(client, context)
    client.identify(transition.scopeUserId)
  } else if (transition.kind === 'reset') {
    client.reset()
    registerAnalyticsEventContext(client, context)
  }
}
