import type { RepoParams } from '@/api/types'
import type { RequestQueueItemResponse } from '@/api/types.generated'
import { updateRequestAttention } from '@/routes/-request-workspace-actions'
import { useNavigate } from '@tanstack/react-router'
import { useRef, useState } from 'react'
import { toast } from 'sonner'
import type { RequestAttentionCommand } from './request-attention-api'
import { requestQueueResource } from './request-queue-cache'

const MESSAGES = {
  claim: 'Request claimed',
  release: 'Claim released',
  restore: 'Request restored',
  settle: 'Request settled',
  snooze: 'Request snoozed',
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
    toast.success(
      MESSAGES[command.action],
      puttingAside
        ? {
            action: {
              label: 'Undo',
              onClick: () => {
                void mutate(
                  requestId,
                  { action: 'restore' },
                  result.attention.activity_version,
                ).then((restored) => {
                  if (restored) void open(requestId)
                })
              },
            },
          }
        : undefined,
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

  return { act, error, pendingId }
}
