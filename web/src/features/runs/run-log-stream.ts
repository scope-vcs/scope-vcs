import type { RunLogResponse } from '@/api/types.generated'
import { apiValidators } from '../../api/validators.generated'
import { normalizeSseLineEndings, reconnectDelay, takeSseMessages } from '../repo-detail/repo-event-stream'

type StreamResult = { stop: boolean }

export async function watchRunLogs({ url, tokenTemplate, getToken, initialCursor, onLog, signal }: {
  url: string
  tokenTemplate: string
  getToken: (options: { template: string }) => Promise<string | null>
  initialCursor: number
  onLog: (log: RunLogResponse) => void
  signal: AbortSignal
}) {
  let cursor = initialCursor
  let reconnects = 0
  while (!signal.aborted) {
    try {
      const result = await streamRunLogs({
        url, tokenTemplate, getToken, cursor,
        onLog: (log) => {
          reconnects = 0
          onLog(log)
          cursor = log.position
        },
        signal,
      })
      if (result.stop || signal.aborted) return
    } catch {
      if (signal.aborted) return
    }
    await waitForReconnect(reconnectDelay(reconnects++, Math.random()), signal)
  }
}

async function streamRunLogs({ url, tokenTemplate, getToken, cursor, onLog, signal }: {
  url: string
  tokenTemplate: string
  getToken: (options: { template: string }) => Promise<string | null>
  cursor: number
  onLog: (log: RunLogResponse) => void
  signal: AbortSignal
}): Promise<StreamResult> {
  const token = await getToken({ template: tokenTemplate })
  const headers = new Headers()
  if (token) headers.set('authorization', `Bearer ${token}`)
  const streamUrl = new URL(url)
  streamUrl.searchParams.set('after', cursor.toString())
  const response = await fetch(streamUrl, { headers, signal })
  if (response.status === 403 || response.status === 404) return { stop: true }
  if (!response.ok || !response.body || response.headers.get('content-type')?.split(';', 1)[0]?.trim() !== 'text/event-stream') {
    throw new Error('Run log stream unavailable.')
  }
  const reader = response.body.getReader()
  const decoder = new TextDecoder()
  let buffer = ''
  try {
    while (!signal.aborted) {
      const chunk = await reader.read()
      if (chunk.done) break
      buffer += decoder.decode(chunk.value, { stream: true })
      buffer = normalizeSseLineEndings(buffer)
      const taken = takeSseMessages(buffer)
      buffer = taken.rest
      for (const message of taken.messages) {
        const lines = message.split('\n')
        const name = lines.find((line) => line.startsWith('event:'))?.slice(6).trim()
        const data = lines.filter((line) => line.startsWith('data:')).map((line) => line.slice(5).trimStart()).join('\n')
        if (!data) continue
        if (name === 'log') {
          const payload: unknown = JSON.parse(data)
          if (!apiValidators.RunLogResponse(payload)) throw new Error('Invalid run log event.')
          if (payload.position > cursor) {
            onLog(payload)
            cursor = payload.position
          }
        } else if (name === 'status') {
          const payload: unknown = JSON.parse(data)
          if (!apiValidators.RunResponse(payload)) throw new Error('Invalid run status event.')
          if (['succeeded', 'failed', 'canceled', 'lost'].includes(payload.state)) {
            return { stop: true }
          }
        } else if (name === 'error') {
          const payload: unknown = JSON.parse(data)
          if (!apiValidators.ErrorResponse(payload)) throw new Error('Invalid run stream error.')
          return { stop: !payload.retryable && payload.code !== 'unauthorized' }
        }
      }
    }
    return { stop: false }
  } finally {
    try { await reader.cancel() } catch {}
    reader.releaseLock()
  }
}

function waitForReconnect(milliseconds: number, signal: AbortSignal) {
  return new Promise<void>((resolve) => {
    if (signal.aborted) return resolve()
    const done = () => {
      clearTimeout(timer)
      signal.removeEventListener('abort', done)
      resolve()
    }
    const timer = setTimeout(done, milliseconds)
    signal.addEventListener('abort', done, { once: true })
  })
}
