import assert from 'node:assert/strict'
import test from 'node:test'
import type { Properties } from './types'
import {
  analyticsEventContext,
  applyAnalyticsIdentityTransition,
} from './client-identity'

const eventContext = analyticsEventContext({
  environment: 'production',
  release: 'web-abc123',
  token: 'phc_project',
})

test('event context is an immutable snapshot of runtime configuration', () => {
  const config = {
    environment: 'test' as const,
    release: 'web-first',
    token: 'phc_project',
  }
  const context = analyticsEventContext(config)
  config.release = 'web-second'

  assert.deepEqual(context, {
    environment: 'test',
    release: 'web-first',
    source: 'browser',
  })
  assert.equal(Object.isFrozen(context), true)
})

test('sign-out restores deployment context after resetting user identity', () => {
  const client = new AnalyticsClient('scope_usr_one', {
    $user_id: 'scope_usr_one',
    ...eventContext,
  })

  applyAnalyticsIdentityTransition(client, null, eventContext)
  client.capture('$pageview')
  client.capture('frontend_error')

  assert.deepEqual(client.events, [
    { event: '$pageview', properties: eventContext },
    { event: 'frontend_error', properties: eventContext },
  ])
  assert.notEqual(client.distinctId, 'scope_usr_one')
  assert.equal(client.properties.$user_id, undefined)
})

test('account switching restores deployment context before identifying the next user', () => {
  const client = new AnalyticsClient('scope_usr_one', {
    $user_id: 'scope_usr_one',
    ...eventContext,
  })

  applyAnalyticsIdentityTransition(client, 'scope_usr_two', eventContext)
  client.capture('frontend_error')

  assert.deepEqual(client.events, [
    {
      event: '$identify',
      properties: { ...eventContext, $user_id: 'scope_usr_two' },
    },
    {
      event: 'frontend_error',
      properties: { ...eventContext, $user_id: 'scope_usr_two' },
    },
  ])
})

class AnalyticsClient {
  events: Array<{ event: string; properties: Properties }> = []
  properties: Properties
  distinctId: string

  constructor(distinctId: string, properties: Properties) {
    this.distinctId = distinctId
    this.properties = properties
  }

  get_distinct_id() {
    return this.distinctId
  }

  get_property(name: string) {
    return this.properties[name]
  }

  identify(scopeUserId: string) {
    this.distinctId = scopeUserId
    this.properties.$user_id = scopeUserId
    this.capture('$identify')
  }

  register(properties: Properties) {
    Object.assign(this.properties, properties)
  }

  reset() {
    this.distinctId = 'new-anonymous-id'
    this.properties = {}
  }

  capture(event: string) {
    this.events.push({ event, properties: { ...this.properties } })
  }
}
