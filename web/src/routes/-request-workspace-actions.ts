import { parseRequestAttentionInput, updateRequestAttentionForRequest } from '@/features/requests/request-attention-api'
import { loadRequestQueueForRequest } from '@/api/requests'
import { parseLoadRequestQueueInput } from '@/api/request-queue-input'
import { createServerFn } from '@tanstack/react-start'

export const loadRequestQueuePage = createServerFn({ method: 'GET' })
  .validator(parseLoadRequestQueueInput)
  .handler(({ data }) => loadRequestQueueForRequest(data))

export const updateRequestAttention = createServerFn({ method: 'POST' })
  .validator(parseRequestAttentionInput)
  .handler(({ data }) => updateRequestAttentionForRequest(data))
