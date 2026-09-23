const MAX_PENDING_EVENTS = 64
const MAX_EVENT_BYTES = 16 * 1024
const REQUEST_TIMEOUT_MS = 2_000
const RETRY_DELAYS_MS = [100, 250] as const

type DeliveryOptions = {
  fetcher?: typeof fetch
  beacon?: (url: string, data: Blob) => boolean
}

export class BoundedDelivery {
  private readonly pending: string[] = []
  private sending = false
  private closed = false
  private current: AbortController | null = null
  private activePayload: string | null = null
  private generation = 0

  constructor(private readonly options: DeliveryOptions = {}) {}

  enqueue(event: unknown) {
    if (this.closed || this.pending.length >= MAX_PENDING_EVENTS) return false
    let payload: string | undefined
    try {
      payload = JSON.stringify(event)
    } catch {
      return false
    }
    if (typeof payload !== 'string') return false
    if (new TextEncoder().encode(payload).byteLength > MAX_EVENT_BYTES) return false
    this.pending.push(payload)
    void this.drain()
    return true
  }

  clear() {
    this.generation++
    this.current?.abort()
    this.activePayload = null
    this.pending.length = 0
  }

  flushOnPageHide() {
    this.closed = true
    this.current?.abort()
    const beacon = this.options.beacon ?? (typeof navigator === 'undefined'
      ? undefined
      : navigator.sendBeacon?.bind(navigator))
    if (beacon) {
      const payloads = this.activePayload === null
        ? this.pending
        : [this.activePayload, ...this.pending]
      for (const payload of payloads) {
        try {
          beacon('/e/e/', new Blob([payload], { type: 'application/json' }))
        } catch {
          // A page closing or an unavailable network may lose analytics.
        }
      }
    }
    this.clear()
  }

  resume() {
    this.closed = false
  }

  private async drain() {
    if (this.sending) return
    this.sending = true
    try {
      while (!this.closed && this.pending.length > 0) {
        const payload = this.pending.shift()!
        this.activePayload = payload
        try {
          await this.deliver(payload)
        } finally {
          this.activePayload = null
        }
      }
    } finally {
      this.sending = false
      if (!this.closed && this.pending.length > 0) void this.drain()
    }
  }

  private async deliver(payload: string) {
    const generation = this.generation
    for (let attempt = 0; attempt <= RETRY_DELAYS_MS.length && !this.closed && generation === this.generation; attempt++) {
      const controller = new AbortController()
      this.current = controller
      const timer = setTimeout(() => controller.abort(), REQUEST_TIMEOUT_MS)
      let retry = true
      try {
        const response = await (this.options.fetcher ?? fetch)('/e/e/', {
          body: payload,
          cache: 'no-store',
          credentials: 'omit',
          headers: { 'Content-Type': 'application/json' },
          method: 'POST',
          referrerPolicy: 'no-referrer',
          signal: controller.signal,
        })
        if (response.ok) return
        retry = response.status === 429 || response.status >= 500
      } catch {
        // Network errors and timeouts are transient until the retry cap.
      } finally {
        clearTimeout(timer)
        if (this.current === controller) this.current = null
      }
      if (this.closed || generation !== this.generation) return
      if (!retry || attempt === RETRY_DELAYS_MS.length) return
      await new Promise(resolve => setTimeout(resolve, RETRY_DELAYS_MS[attempt]))
    }
  }
}
