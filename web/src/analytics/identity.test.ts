import assert from 'node:assert/strict'
import test from 'node:test'
import {
  expectedIdentityKey,
  identityTransition,
  resolveAnalyticsIdentity,
} from './identity'

test('signed-in anonymous visitors identify with the internal Scope user ID', () => {
  assert.deepEqual(identityTransition({
    currentDistinctId: 'anonymous-id',
    isSignedIn: true,
    persistedUserId: undefined,
    scopeUserId: 'scope_usr_123',
  }), {
    kind: 'identify',
    scopeUserId: 'scope_usr_123',
  })
})

test('sign-out resets an identified browser but not a new anonymous visit', () => {
  assert.deepEqual(identityTransition({
    currentDistinctId: 'scope_usr_123',
    isSignedIn: false,
    persistedUserId: 'scope_usr_123',
  }), { kind: 'reset' })
  assert.deepEqual(identityTransition({
    currentDistinctId: 'anonymous-id',
    isSignedIn: false,
    persistedUserId: undefined,
  }), { kind: 'none' })
})

test('sign-out resets a Scope distinct ID even without a persisted user property', () => {
  assert.deepEqual(identityTransition({
    currentDistinctId: 'scope_usr_123',
    isSignedIn: false,
    persistedUserId: undefined,
  }), { kind: 'reset' })
})

test('already identified visitors do not emit a duplicate identify', () => {
  assert.deepEqual(identityTransition({
    currentDistinctId: 'scope_usr_123',
    isSignedIn: true,
    persistedUserId: 'scope_usr_123',
    scopeUserId: 'scope_usr_123',
  }), { kind: 'none' })
})

test('account switching resets before identifying the next Scope user', () => {
  assert.deepEqual(identityTransition({
    currentDistinctId: 'scope_usr_one',
    isSignedIn: true,
    persistedUserId: 'scope_usr_one',
    scopeUserId: 'scope_usr_two',
  }), {
    kind: 'reset_and_identify',
    scopeUserId: 'scope_usr_two',
  })
})

const signedIn = {
  clerkUserId: 'clerk_user_123',
  isLoaded: true,
  isSignedIn: true,
}

test('identity resolves the internal Scope user ID from the resolved session', () => {
  assert.deepEqual(resolveAnalyticsIdentity({
    ...signedIn,
    scopeUserId: 'scope_usr_123',
    sessionResolved: true,
  }), {
    identityKey: 'identified:clerk_user_123',
    scopeUserId: 'scope_usr_123',
  })
})

test('a signed-in viewer stays unresolved until the session resource publishes', () => {
  assert.equal(resolveAnalyticsIdentity({
    ...signedIn,
    scopeUserId: null,
    sessionResolved: false,
  }), null)
  assert.equal(expectedIdentityKey(signedIn), 'identified:clerk_user_123')
})

test('a session that resolves without a Scope account identifies nobody', () => {
  assert.deepEqual(resolveAnalyticsIdentity({
    ...signedIn,
    scopeUserId: null,
    sessionResolved: true,
  }), {
    identityKey: 'identified:clerk_user_123',
    scopeUserId: null,
  })
})

test('signed-out viewers resolve anonymously without reading a session', () => {
  assert.deepEqual(resolveAnalyticsIdentity({
    clerkUserId: null,
    isLoaded: true,
    isSignedIn: false,
    scopeUserId: null,
    sessionResolved: false,
  }), { identityKey: 'anonymous', scopeUserId: null })
})

test('an unloaded auth state resolves to no identity at all', () => {
  const pending = {
    clerkUserId: null,
    isLoaded: false,
    isSignedIn: false,
    scopeUserId: null,
    sessionResolved: false,
  }
  assert.equal(resolveAnalyticsIdentity(pending), null)
  assert.equal(expectedIdentityKey(pending), null)
})
