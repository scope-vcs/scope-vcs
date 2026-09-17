import type { RepoParams } from '@/api/types'
import type { RequestQueueItemResponse } from '@/api/types.generated'
import { updateRequestAttention } from '@/routes/-request-workspace-actions'
import { useNavigate } from '@tanstack/react-router'
import { useEffect, useRef, useState } from 'react'
import { toast } from 'sonner'
import type { RequestAttentionCommand } from './request-attention-api'
import {
  applyAttentionMoves,
  isInstantCommand,
  queueReflectsMove,
  type RequestAttentionMove,
  type RequestInstantCommand,
} from './request-attention-moves'
import type { RequestQueuePages } from './request-list-model'
import { requestQueueResource } from './request-queue-cache'

/**
 * Settle, snooze and restore move the row in the browser at once and tell the
 * server afterwards. Claim and release wait for the server, since they change
 * who owns the review.
 */
export function useRequestAttentionActions(
  identity: string | null,
  params: RepoParams,
  loadedPages: RequestQueuePages | undefined,
  selectedId?: string,
) {
  const navigate = useNavigate()
  const inFlight = useRef(false)
  const selectedRef = useRef(selectedId)
  selectedRef.current = selectedId
  const [pendingId, setPendingId] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [moves, setMoves] = useState<RequestAttentionMove[]>([])

  // A move is already ignored once the loaded queue reflects it; this only
  // stops finished moves from piling up.
  useEffect(() => {
    if (!loadedPages || !moves.some((move) => queueReflectsMove(loadedPages, move))) return
    setMoves((current) => current.filter((move) => !queueReflectsMove(loadedPages, move)))
  }, [loadedPages, moves])

  function open(requestId?: string) {
    return requestId
      ? navigate({ to: '/$owner/$repo/requests/$requestId', params: { ...params, requestId } })
      : navigate({ to: '/$owner/$repo/requests', params })
  }

  async function mutate(requestId: string, command: RequestAttentionCommand, version: number) {
    if (!identity) return null
    setError(null)
    try {
      return await updateRequestAttention({
        data: { ...params, request_id: requestId, ...command, expected_activity_version: version },
      })
    } catch (cause) {
      const message = cause instanceof Error ? cause.message : 'Could not update request attention.'
      setError(message)
      toast.error(message)
      return null
    } finally {
      requestQueueResource.invalidate(identity)
    }
  }

  async function moveNow(item: RequestQueueItemResponse, command: RequestInstantCommand) {
    const requestId = item.request.id
    if (!identity || moves.some((move) => move.item.request.id === requestId && !move.confirmed)) return
    const move: RequestAttentionMove = {
      item,
      command,
      atUnix: Math.floor(Date.now() / 1_000),
      confirmed: null,
    }
    const loaded = requestQueueResource.peek(identity)?.pages
    const before = loaded ? applyAttentionMoves(loaded, moves).active.requests : []
    setMoves((current) => [...current.filter((entry) => entry.item.request.id !== requestId), move])
    let movedOnTo: { id: string | undefined } | null = null
    if (command.action !== 'restore' && selectedId === requestId) {
      const index = Math.max(0, before.findIndex((row) => row.request.id === requestId))
      const remaining = before.filter((row) => row.request.id !== requestId)
      movedOnTo = { id: remaining[Math.min(index, remaining.length - 1)]?.request.id }
      void open(movedOnTo.id)
    }
    const confirmed = await mutate(requestId, command, item.attention.activity_version)
    // A refused move puts the row back where the server still has it, and the
    // viewer back on it unless they have gone somewhere else since.
    if (!confirmed && movedOnTo && selectedRef.current === movedOnTo.id) void open(requestId)
    setMoves((current) =>
      confirmed
        ? current.map((entry) => (entry === move ? { ...move, confirmed } : entry))
        : current.filter((entry) => entry !== move),
    )
  }

  async function act(item: RequestQueueItemResponse, command: RequestAttentionCommand) {
    if (isInstantCommand(command)) return moveNow(item, command)
    if (!identity || inFlight.current) return
    const requestId = item.request.id
    // Claim and release can remove the record a move is waiting on.
    setMoves((current) => current.filter((move) => move.item.request.id !== requestId))
    inFlight.current = true
    setPendingId(requestId)
    let result
    try {
      result = await mutate(requestId, command, item.attention.activity_version)
    } finally {
      inFlight.current = false
      setPendingId(null)
    }
    if (result && command.action === 'claim') await open(requestId)
  }

  return { act, error, moves, pendingId }
}
