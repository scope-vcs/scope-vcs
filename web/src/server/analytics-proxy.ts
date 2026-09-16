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
}

type AnalyticsDeliveryObservation = {
  durationMs: number
  status: number
}

export async function proxyAnalyticsCapture(
  request: Request,
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

    const sourceUrl = new URL(request.url)
    const upstreamUrl = new URL(POSTHOG_CAPTURE_UPSTREAM)
    upstreamUrl.search = sourceUrl.search

    try {
      const upstream = await fetchUpstream(upstreamUrl, {
        body: body.buffer,
        headers: captureHeaders(request.headers),
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
      console.info('[analytics-proxy] delivery', observation)
    }
  } catch {
    // Delivery reporting must not alter the proxy response.
  }
}

function captureHeaders(headers: Headers) {
  const forwarded = new Headers()
  for (const name of ['content-encoding', 'content-type']) {
    const value = headers.get(name)
    if (value) forwarded.set(name, value)
  }
  return forwarded
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
