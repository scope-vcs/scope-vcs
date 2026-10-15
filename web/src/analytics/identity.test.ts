import assert from 'node:assert/strict'
import test from 'node:test'
import {
  expectedIdentityKey,
  identityTransition,
  resolveAnalyticsIdentity,
} from './identity'

test('identity transitions identify, reset or switch the browser without duplicate identifies', () => {
  const cases: [string, Parameters<typeof identityTransition>[0], ReturnType<typeof identityTransition>][] = [
    ['signed-in anonymous visitor identifies with the Scope user ID', {
      currentDistinctId: 'anonymous-id', isSignedIn: true, identifiedUserId: undefined, scopeUserId: 'scope_usr_123',
    }, { kind: 'identify', scopeUserId: 'scope_usr_123' }],
    ['sign-out resets an identified browser', {
      currentDistinctId: 'scope_usr_123', isSignedIn: false, identifiedUserId: 'scope_usr_123',
    }, { kind: 'reset' }],
    ['a new anonymous visit is left alone', {
      currentDistinctId: 'anonymous-id', isSignedIn: false, identifiedUserId: undefined,
    }, { kind: 'none' }],
    ['sign-out resets a Scope distinct ID without a persisted user property', {
      currentDistinctId: 'scope_usr_123', isSignedIn: false, identifiedUserId: undefined,
    }, { kind: 'reset' }],
    ['an already identified visitor is not identified again', {
      currentDistinctId: 'scope_usr_123', isSignedIn: true, identifiedUserId: 'scope_usr_123', scopeUserId: 'scope_usr_123',
    }, { kind: 'none' }],
    ['account switching resets before identifying the next user', {
      currentDistinctId: 'scope_usr_one', isSignedIn: true, identifiedUserId: 'scope_usr_one', scopeUserId: 'scope_usr_two',
    }, { kind: 'reset_and_identify', scopeUserId: 'scope_usr_two' }],
  ]
  for (const [name, input, expected] of cases) assert.deepEqual(identityTransition(input), expected, name)
})

test('identity resolves from auth and the published session', () => {
  const signedIn = { clerkUserId: 'clerk_user_123', isLoaded: true, isSignedIn: true }
  const signedOut = { clerkUserId: null, isLoaded: true, isSignedIn: false }
  const unloaded = { clerkUserId: null, isLoaded: false, isSignedIn: false }
  const cases: [string, Parameters<typeof resolveAnalyticsIdentity>[0], ReturnType<typeof resolveAnalyticsIdentity>][] = [
    ['a resolved session supplies the Scope user ID', {
      ...signedIn, scopeUserId: 'scope_usr_123', sessionResolved: true,
    }, { identityKey: 'identified:clerk_user_123', scopeUserId: 'scope_usr_123' }],
    ['a signed-in viewer waits for the session resource', {
      ...signedIn, scopeUserId: null, sessionResolved: false,
    }, null],
    ['a session without a Scope account identifies nobody', {
      ...signedIn, scopeUserId: null, sessionResolved: true,
    }, { identityKey: 'identified:clerk_user_123', scopeUserId: null }],
    ['a signed-out viewer resolves anonymously without a session', {
      ...signedOut, scopeUserId: null, sessionResolved: false,
    }, { identityKey: 'anonymous', scopeUserId: null }],
    ['an unloaded auth state resolves to no identity', {
      ...unloaded, scopeUserId: null, sessionResolved: false,
    }, null],
  ]
  for (const [name, input, expected] of cases) assert.deepEqual(resolveAnalyticsIdentity(input), expected, name)
  assert.equal(expectedIdentityKey(signedIn), 'identified:clerk_user_123')
  assert.equal(expectedIdentityKey(unloaded), null)
})
