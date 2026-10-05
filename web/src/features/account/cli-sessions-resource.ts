import type { CliSessionsResponse } from '@/api/types.generated'
import { createCachedResource } from '@/lib/cached-resource'

export const cliSessionsResource = createCachedResource<CliSessionsResponse>({ maxEntries: 1 })

let revocationGeneration = 0

export function cliSessionsRevocationGeneration() {
  return revocationGeneration
}

export function cliSessionsIdentity(viewerId: string) {
  return `cli-sessions\0${viewerId}`
}

export function retainRevokedCliSession(identity: string, sessionId: string) {
  revocationGeneration += 1
  const current = cliSessionsResource.peek(identity)
  if (!current) return
  cliSessionsResource.write(identity, {
    sessions: current.sessions.filter((session) => session.id !== sessionId),
  })
}
