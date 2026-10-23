import { serverLog } from './log'
import { isIP } from 'node:net'
import { sanitizeCapture } from '../analytics/privacy'
import type { CaptureResult } from '../analytics/types'
import type { AnalyticsRuntimeConfig } from '../analytics/config'
import { publicRequestOrigin } from './public-origin'

const POSTHOG_CAPTURE_UPSTREAM = 'https://us.i.posthog.com/e/'
const MAX_REQUEST_BYTES = 1024 * 1024
const MAX_RESPONSE_BYTES = 64 * 1024
const REQUEST_TIMEOUT_MS = 5_000

type FetchAnalyticsUpstream = (
  input: string | URL | Request,
  init?: RequestInit,
) => Promise<Response>

export type AnalyticsProxyOptions = {
  fetchUpstream?: FetchAnalyticsUpstream
  observeDelivery?: (observation: AnalyticsDeliveryObservation) => void
  timeoutMs?: number
  runtime?: Readonly<Record<string, string | undefined>>
}

type AnalyticsDeliveryObservation = {
  durationMs: number
  status: number
}

export async function proxyAnalyticsCapture(
  request: Request,
  config: AnalyticsRuntimeConfig,
  options: AnalyticsProxyOptions = {},
) {
  const startedAt = performance.now()
  const fetchUpstream = options.fetchUpstream ?? fetch
  const finish = (response: Response) => {
    observeDelivery(options.observeDelivery, {
      durationMs: Math.max(0, Math.round(performance.now() - startedAt)),
      status: response.status,
    })
    return response
  }
  const contentLength = request.headers.get('content-length')
  if (contentLength && /^\d+$/.test(contentLength) && Number(contentLength) > MAX_REQUEST_BYTES) {
    return finish(proxyResponse('Analytics request is too large.', 413))
  }
  if (request.headers.has('content-encoding')) {
    return finish(proxyResponse('Unsupported analytics encoding.', 415))
  }

  const controller = new AbortController()
  const timeout = setTimeout(
    () => controller.abort(),
    options.timeoutMs ?? REQUEST_TIMEOUT_MS,
  )
  try {
    let body: Uint8Array<ArrayBuffer> | null
    try {
      body = await readBoundedBody(request, MAX_REQUEST_BYTES, controller.signal)
    } catch {
      return finish(controller.signal.aborted
        ? proxyResponse('Analytics request timed out.', 408)
        : proxyResponse('Invalid analytics request.', 400))
    }
    if (!body) return finish(proxyResponse('Analytics request is too large.', 413))

    const runtime = options.runtime ?? process.env
    const clientIp = analyticsClientIp(request, runtime)
    const capture = prepareCapture(body, publicRequestOrigin(request.url, runtime.RAILWAY_ENVIRONMENT_ID), config, clientIp)
    if (!capture) return finish(proxyResponse('Invalid analytics event.', 400))
    const headers = new Headers({ 'content-type': 'application/json' })
    if (clientIp) headers.set('x-forwarded-for', clientIp)

    try {
      const upstream = await fetchUpstream(POSTHOG_CAPTURE_UPSTREAM, {
        body: JSON.stringify(capture),
        headers,
        method: 'POST',
        redirect: 'manual',
        signal: controller.signal,
      })
      const response = await upstreamResponse(upstream, controller.signal)
      return finish(response ?? proxyResponse('Analytics upstream response is too large.', 502))
    } catch {
      return finish(proxyResponse(
        controller.signal.aborted ? 'Analytics upstream timed out.' : 'Analytics upstream unavailable.',
        controller.signal.aborted ? 504 : 502,
      ))
    }
  } finally {
    clearTimeout(timeout)
  }
}

function observeDelivery(
  observer: AnalyticsProxyOptions['observeDelivery'],
  observation: AnalyticsDeliveryObservation,
) {
  try {
    if (observer) {
      observer(observation)
    } else {
      serverLog('info', '[analytics-proxy] delivery', observation)
    }
  } catch {}
}

function analyticsClientIp(request: Request, runtime: NonNullable<AnalyticsProxyOptions['runtime']>) {
  if (
    runtime.SCOPE_ANALYTICS_GEOIP !== 'enabled'
    || !runtime.RAILWAY_ENVIRONMENT_ID
    || request.headers.get('sec-gpc') === '1'
    || request.headers.get('dnt') === '1'
  ) return null

  const address = request.headers.get('x-real-ip')?.trim()
  return address && isIP(address) ? address : null
}

function prepareCapture(
  body: Uint8Array,
  origin: string,
  config: AnalyticsRuntimeConfig,
  clientIp: string | null,
) {
  try {
    const capture: unknown = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(body))
    if (
      !capture || typeof capture !== 'object'
      || !('event' in capture) || typeof capture.event !== 'string'
      || !('uuid' in capture) || typeof capture.uuid !== 'string'
      || !('properties' in capture) || !capture.properties
      || typeof capture.properties !== 'object' || Array.isArray(capture.properties)
      || !('distinct_id' in capture.properties)
      || typeof capture.properties.distinct_id !== 'string'
      || !capture.properties.distinct_id
    ) return null

    const event: CaptureResult = {
      event: capture.event,
      uuid: capture.uuid,
      properties: capture.properties as CaptureResult['properties'],
      ...('timestamp' in capture && typeof capture.timestamp === 'string'
        ? { timestamp: capture.timestamp } : {}),
    }
    const sanitized = sanitizeCapture(event, origin)
    if (!sanitized) return null
    sanitized.properties.token = config.token
    sanitized.properties.environment = config.environment
    sanitized.properties.release = config.release
    sanitized.properties.source = 'browser'
    sanitized.properties.$geoip_disable = !clientIp
    if (clientIp) sanitized.properties.$ip = clientIp
    return sanitized
  } catch {
    return null
  }
}

async function upstreamResponse(upstream: Response, signal: AbortSignal) {
  const contentLength = upstream.headers.get('content-length')
  if (contentLength && /^\d+$/.test(contentLength) && Number(contentLength) > MAX_RESPONSE_BYTES) {
    await upstream.body?.cancel()
    return null
  }

  const body = upstream.body
    ? await readBoundedStream(upstream.body, MAX_RESPONSE_BYTES, signal)
    : new Uint8Array()
  if (!body) return null

  const headers = new Headers({ 'cache-control': 'no-store' })
  for (const name of ['content-type', 'retry-after']) {
    const value = upstream.headers.get(name)
    if (value) headers.set(name, value)
  }

  return new Response(body.byteLength > 0 ? body.buffer : null, {
    headers,
    status: upstream.status,
    statusText: upstream.statusText,
  })
}

function proxyResponse(message: string, status: number) {
  return new Response(message, {
    headers: {
      'cache-control': 'no-store',
      'content-type': 'text/plain; charset=utf-8',
    },
    status,
  })
}

async function readBoundedBody(
  request: Request,
  maximumBytes: number,
  signal: AbortSignal,
): Promise<Uint8Array<ArrayBuffer> | null> {
  return request.body
    ? readBoundedStream(request.body, maximumBytes, signal)
    : new Uint8Array()
}

async function readBoundedStream(
  stream: ReadableStream<Uint8Array>,
  maximumBytes: number,
  signal: AbortSignal,
): Promise<Uint8Array<ArrayBuffer> | null> {
  const chunks: Uint8Array[] = []
  const reader = stream.getReader()
  let byteLength = 0
  const cancelRead = () => {
    void reader.cancel(signal.reason).catch(() => {})
  }

  signal.addEventListener('abort', cancelRead, { once: true })
  try {
    if (signal.aborted) cancelRead()
    signal.throwIfAborted()

    while (true) {
      const { done, value } = await reader.read()
      signal.throwIfAborted()
      if (done) break
      byteLength += value.byteLength
      if (byteLength > maximumBytes) {
        await reader.cancel()
        return null
      }
      chunks.push(value)
    }
  } finally {
    signal.removeEventListener('abort', cancelRead)
    reader.releaseLock()
  }

  const body = new Uint8Array(byteLength)
  let offset = 0
  for (const chunk of chunks) {
    body.set(chunk, offset)
    offset += chunk.byteLength
  }
  return body
}
