type IdentityTransition =
  | { kind: 'identify'; scopeUserId: string }
  | { kind: 'reset_and_identify'; scopeUserId: string }
  | { kind: 'reset' }
  | { kind: 'none' }

type ResolvedAnalyticsIdentity = {
  identityKey: string
  scopeUserId: string | null
}

type AnalyticsViewer = {
  clerkUserId: string | null
  isLoaded: boolean
  isSignedIn: boolean
}

const anonymousIdentityKey = 'anonymous'

// The identity a viewer will settle on, known before the account session that
// carries its Scope user ID resolves.
export function expectedIdentityKey(viewer: AnalyticsViewer) {
  if (!viewer.isLoaded) return null
  return viewer.isSignedIn && viewer.clerkUserId
    ? identifiedKey(viewer.clerkUserId)
    : anonymousIdentityKey
}

// Null means the identity is still unresolved: nothing may be attributed to the
// viewer until the account session the resource owns publishes a value.
export function resolveAnalyticsIdentity(
  viewer: AnalyticsViewer & {
    scopeUserId: string | null
    sessionResolved: boolean
  },
): ResolvedAnalyticsIdentity | null {
  const identityKey = expectedIdentityKey(viewer)
  if (identityKey === null) return null
  if (identityKey === anonymousIdentityKey) {
    return { identityKey, scopeUserId: null }
  }
  return viewer.sessionResolved
    ? { identityKey, scopeUserId: viewer.scopeUserId }
    : null
}

export function identityTransition(input: {
  currentDistinctId: string
  isSignedIn: boolean
  persistedUserId: unknown
  scopeUserId?: string | null
}): IdentityTransition {
  if (!input.isSignedIn) {
    const hasScopeIdentity = input.currentDistinctId.startsWith('scope_usr_')
      || Boolean(input.persistedUserId)
    return hasScopeIdentity ? { kind: 'reset' } : { kind: 'none' }
  }

  if (
    input.scopeUserId
    && input.currentDistinctId !== input.scopeUserId
  ) {
    const replacingScopeIdentity = input.currentDistinctId.startsWith('scope_usr_')
      || Boolean(input.persistedUserId)
    return {
      kind: replacingScopeIdentity ? 'reset_and_identify' : 'identify',
      scopeUserId: input.scopeUserId,
    }
  }

  return { kind: 'none' }
}

function identifiedKey(clerkUserId: string) {
  return `identified:${clerkUserId}`
}
