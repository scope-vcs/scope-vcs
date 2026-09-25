import { BoundedDelivery } from './delivery'
import { createPrivacyBoundary } from './privacy'
import type { Properties } from './types'

// Identities live in memory only, so analytics stores nothing on the device.
export class AnalyticsClient {
  private readonly delivery = new BoundedDelivery()
  private readonly sanitize: ReturnType<typeof createPrivacyBoundary>
  private readonly disabled: boolean
  private distinctId: string = crypto.randomUUID()
  private properties: Properties = {}
  // Whether an event was queued under the current anonymous ID. Without one
  // there is no anonymous history to merge, so identify switches silently.
  private anonymousEventQueued = false

  constructor(private readonly token: string, origin: string) {
    this.disabled = (navigator as Navigator & { globalPrivacyControl?: boolean }).globalPrivacyControl === true
      || navigator.doNotTrack === '1'
      || navigator.doNotTrack === 'yes'
      || (window as Window & { doNotTrack?: string }).doNotTrack === '1'
    this.sanitize = createPrivacyBoundary(origin)
    if (!this.disabled) {
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
    if (this.anonymousEventQueued) this.capture('$identify', { $anon_distinct_id: previousId })
    this.anonymousEventQueued = false
  }

  reset() {
    this.delivery.clear()
    this.distinctId = crypto.randomUUID()
    this.properties = {}
    this.anonymousEventQueued = false
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
    if (capture && this.delivery.enqueue(capture) && !this.properties.$user_id) {
      this.anonymousEventQueued = true
    }
  }
}
