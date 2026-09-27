import { createBoundedCache } from '../lib/bounded-cache'

// Minting a Clerk template token is a round trip to api.clerk.com. The API
// trusts a minted token until its `exp`, so reusing it for the same verified
// session until shortly before then keeps the same revocation window.
const reuseMarginMs = 30_000
const defaultMaxEntries = 1_000

export type ClerkApiTokenOwner = {
  sessionId: string
  template: string
  userId: string
}

type CachedToken = {
  reuseUntilMs: number
  token: string
}

export function createClerkApiTokenCache({
  maxEntries = defaultMaxEntries,
  now = Date.now,
}: { maxEntries?: number; now?: () => number } = {}) {
  const tokens = createBoundedCache<string, CachedToken>({ maxEntries })
  const mints = new Map<string, Promise<string | null>>()

  async function mintAndStore(key: string, mint: () => Promise<string | null>) {
    const token = await mint()
    const expiresAtMs = token ? tokenExpiryMs(token) : undefined
    if (token && expiresAtMs !== undefined) {
      tokens.set(key, { reuseUntilMs: expiresAtMs - reuseMarginMs, token })
    }
    return token
  }

  return {
    read(owner: ClerkApiTokenOwner, mint: () => Promise<string | null>) {
      const key = ownerKey(owner)
      const cached = tokens.get(key)
      if (cached && cached.reuseUntilMs > now()) {
        return Promise.resolve(cached.token)
      }

      const pending = mints.get(key)
      if (pending) return pending

      const minted = mintAndStore(key, mint).finally(() => mints.delete(key))
      mints.set(key, minted)
      return minted
    },
  }
}

function ownerKey({ sessionId, template, userId }: ClerkApiTokenOwner) {
  return JSON.stringify([template, sessionId, userId])
}

function tokenExpiryMs(token: string) {
  const payload = token.split('.')[1]
  if (!payload) return undefined
  try {
    const { exp } = JSON.parse(atob(payload.replace(/-/g, '+').replace(/_/g, '/'))) as {
      exp?: unknown
    }
    return typeof exp === 'number' && Number.isFinite(exp) ? exp * 1000 : undefined
  } catch {
    return undefined
  }
}
