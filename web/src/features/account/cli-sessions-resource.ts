import type { CliSessionsResponse } from '@/api/types.generated'
import { createCachedResource } from '../../lib/cached-resource'
import { useCachedResource } from '../../lib/use-cached-resource'
import { onViewerChange } from '../../lib/viewer-state'
import { useEffect, useMemo } from 'react'

const cliSessionsResource = createCachedResource<CliSessionsResponse>({ maxEntries: 1 })

let revocationGeneration = 0
onViewerChange(() => { revocationGeneration += 1 })

function cliSessionsIdentity(viewerId: string) {
  return `cli-sessions\0${viewerId}`
}

export type CliSessionsHandoff = {
  viewerId: string | null
  sessions: CliSessionsResponse
  revocationGeneration: number
}

export async function loadCliSessionsHandoff(
  load: () => Promise<{ viewerId: string | null; sessions: CliSessionsResponse }>,
): Promise<CliSessionsHandoff> {
  const startedAtGeneration = revocationGeneration
  return { ...await load(), revocationGeneration: startedAtGeneration }
}

function matchesCurrentViewer(viewerId: string | null, handoff: CliSessionsHandoff): viewerId is string {
  return viewerId !== null && handoff.viewerId === viewerId &&
    handoff.revocationGeneration === revocationGeneration
}

export function acceptCliSessionsHandoff(viewerId: string | null, handoff: CliSessionsHandoff) {
  if (!matchesCurrentViewer(viewerId, handoff)) return null
  cliSessionsResource.write(cliSessionsIdentity(viewerId), handoff.sessions)
  return handoff.sessions
}

export function readRetainedCliSessions(viewerId: string) {
  return cliSessionsResource.peek(cliSessionsIdentity(viewerId))
}

export function useCliSessionsResource({ viewerId, handoff, load }: {
  viewerId: string | null
  handoff: CliSessionsHandoff
  load: (signal: AbortSignal) => Promise<CliSessionsResponse>
}) {
  const identity = viewerId === null ? null : cliSessionsIdentity(viewerId)
  const initialValue = matchesCurrentViewer(viewerId, handoff) ? handoff.sessions : null
  const resourceForHandoff = useMemo(() => ({
    ...cliSessionsResource,
    seed(identity: string, value: CliSessionsResponse, version?: string) {
      if (matchesCurrentViewer(viewerId, handoff)) {
        cliSessionsResource.seed(identity, value, version)
      }
    },
  }), [viewerId, handoff])
  const resource = useCachedResource({
    fallbackError: 'CLI sessions are unavailable.',
    identity,
    initialValue,
    load,
    resource: resourceForHandoff,
  })
  useEffect(() => {
    acceptCliSessionsHandoff(viewerId, handoff)
  }, [viewerId, handoff])
  return { resource, sessions: resource.value?.sessions ??
    (viewerId === null ? null : readRetainedCliSessions(viewerId)?.sessions) ?? initialValue?.sessions ?? [] }
}

export function retainRevokedCliSession(viewerId: string, sessionId: string) {
  revocationGeneration += 1
  const identity = cliSessionsIdentity(viewerId)
  const current = cliSessionsResource.peek(identity)
  if (!current) return
  cliSessionsResource.write(identity, {
    sessions: current.sessions.filter((session) => session.id !== sessionId),
  })
}
