import type { RepoParams } from '@/api/types'
import type { RequestQueueItemResponse } from '@/api/types.generated'
import { updateRequestAttention } from '@/routes/-request-workspace-actions'
import { useNavigate } from '@tanstack/react-router'
import { useEffect, useRef, useState } from 'react'
import { toast } from 'sonner'
import type { RequestAttentionCommand } from './request-attention-api'
import { requestQueueResource } from './request-queue-cache'

const UNDO_WINDOW_MS = 8_000

/** A settle or snooze the viewer can still take back. */
export type RequestUndoableAction = {
  item: RequestQueueItemResponse
  label: string
  version: number
}

function undoLabel(command: RequestAttentionCommand) {
  if (command.action === 'snooze') {
    const until = new Date(command.until_unix * 1000)
    return `Snoozed until ${until.toLocaleString(undefined, { weekday: 'short', hour: 'numeric', minute: '2-digit' })}`
  }
  return 'Settled for now'
}

export function useRequestAttentionActions(
  identity: string | null,
  params: RepoParams,
  selectedId?: string,
) {
  const navigate = useNavigate()
  const inFlight = useRef(false)
  const [pendingId, setPendingId] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [undoable, setUndoable] = useState<RequestUndoableAction | null>(null)

  useEffect(() => {
    if (!undoable) return
    const timer = setTimeout(() => setUndoable(null), UNDO_WINDOW_MS)
    return () => clearTimeout(timer)
  }, [undoable])

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

  async function act(item: RequestQueueItemResponse, command: RequestAttentionCommand) {
    if (!identity || inFlight.current) return
    const requestId = item.request.id
    const active = requestQueueResource.peek(identity)?.pages.active.requests ?? []
    inFlight.current = true
    setPendingId(requestId)
    let result
    try {
      result = await mutate(requestId, command, item.attention.activity_version)
    } finally {
      inFlight.current = false
      setPendingId(null)
    }
    if (!result) return
    const puttingAside = command.action === 'settle' || command.action === 'snooze'
    setUndoable(
      puttingAside
        ? { item, label: undoLabel(command), version: result.attention.activity_version }
        : null,
    )
    if (command.action === 'claim') await open(requestId)
    else if (puttingAside && selectedId === requestId) {
      const index = Math.max(
        0,
        active.findIndex((row) => row.request.id === requestId),
      )
      const remaining = active.filter((row) => row.request.id !== requestId)
      await open(remaining[Math.min(index, remaining.length - 1)]?.request.id)
    }
  }

  async function undo() {
    if (!undoable || inFlight.current) return
    const requestId = undoable.item.request.id
    inFlight.current = true
    setPendingId(requestId)
    let restored
    try {
      restored = await mutate(requestId, { action: 'restore' }, undoable.version)
    } finally {
      inFlight.current = false
      setPendingId(null)
    }
    setUndoable(null)
    if (restored) await open(requestId)
  }

  return { act, error, pendingId, undo, undoable }
}
