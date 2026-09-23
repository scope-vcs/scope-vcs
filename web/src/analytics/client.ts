import { BoundedDelivery } from './delivery'
import { createPrivacyBoundary } from './privacy'
import type { Properties } from './types'

const DISTINCT_ID_KEY = 'scope_analytics_distinct_id'
const USER_ID_KEY = 'scope_analytics_user_id'

export class AnalyticsClient {
  private readonly delivery = new BoundedDelivery()
  private readonly sanitize: ReturnType<typeof createPrivacyBoundary>
  private readonly disabled: boolean
  private distinctId: string
  private properties: Properties = {}

  constructor(private readonly token: string, origin: string) {
    this.disabled = navigator.doNotTrack === '1'
      || navigator.doNotTrack === 'yes'
      || (window as Window & { doNotTrack?: string }).doNotTrack === '1'
    this.sanitize = createPrivacyBoundary(origin)
    this.distinctId = this.disabled ? crypto.randomUUID() : readStored(DISTINCT_ID_KEY) ?? crypto.randomUUID()
    if (!this.disabled) {
      const userId = readStored(USER_ID_KEY)
      if (userId) this.properties.$user_id = userId
      store(DISTINCT_ID_KEY, this.distinctId)
      window.addEventListener('pagehide', () => this.delivery.flushOnPageHide())
      window.addEventListener('pageshow', () => this.delivery.resume())
    }
  }

  get_distinct_id() {
    return this.distinctId
  }

  get_property(name: string) {
    return this.properties[name]
  }

  register(properties: Properties) {
    Object.assign(this.properties, properties)
  }

  identify(scopeUserId: string) {
    if (this.disabled || scopeUserId === this.distinctId) return
    const previousId = this.distinctId
    this.distinctId = scopeUserId
    this.properties.$user_id = scopeUserId
    store(DISTINCT_ID_KEY, scopeUserId)
    store(USER_ID_KEY, scopeUserId)
    this.capture('$identify', { $anon_distinct_id: previousId })
  }

  reset() {
    this.delivery.clear()
    this.distinctId = crypto.randomUUID()
    this.properties = {}
    if (!this.disabled) {
      removeStored(USER_ID_KEY)
      store(DISTINCT_ID_KEY, this.distinctId)
    }
  }

  capture(event: string, properties: Properties = {}) {
    if (this.disabled) return
    const capture = this.sanitize({
      event,
      properties: {
        ...this.properties,
        ...properties,
        distinct_id: this.distinctId,
        token: this.token,
        $process_person_profile: Boolean(this.properties.$user_id),
      },
      uuid: crypto.randomUUID(),
      timestamp: new Date().toISOString(),
    })
    if (capture) this.delivery.enqueue(capture)
  }
}

function readStored(key: string) {
  try { return localStorage.getItem(key) } catch { return null }
}

function store(key: string, value: string) {
  try { localStorage.setItem(key, value) } catch { /* storage may be unavailable */ }
}

function removeStored(key: string) {
  try { localStorage.removeItem(key) } catch { /* storage may be unavailable */ }
}
