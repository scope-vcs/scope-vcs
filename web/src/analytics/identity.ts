type IdentityTransition =
  | { kind: 'identify'; scopeUserId: string }
  | { kind: 'reset_and_identify'; scopeUserId: string }
  | { kind: 'reset' }
  | { kind: 'none' }

type ResolvedAnalyticsIdentity = {
  identityKey: string
  scopeUserId: string | null
}

export async function resolveAnalyticsIdentity(
  clerkUserId: string,
  loadAccount: () => Promise<{ user: { id: string } | null } | null>,
): Promise<ResolvedAnalyticsIdentity> {
  const account = await loadAccount()
  return {
    identityKey: identifiedKey(clerkUserId),
    scopeUserId: account?.user?.id ?? null,
  }
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

export function identifiedKey(clerkUserId: string) {
  return `identified:${clerkUserId}`
}
