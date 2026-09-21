type Priority = 0 | 1 | null
type Registration = { near: boolean; visible: boolean; notify: (priority: Priority) => void }
const registrations = new Map<Element, Registration>()
let nearbyObserver: IntersectionObserver | null = null
let visibleObserver: IntersectionObserver | null = null

// All diagram blocks share two observers, regardless of discussion length.
export function observeRequestMermaid(element: Element, notify: Registration['notify']) {
  if (!nearbyObserver) {
    const update = (entries: IntersectionObserverEntry[], field: 'near' | 'visible') => {
      for (const entry of entries) {
        const registration = registrations.get(entry.target)
        if (!registration) continue
        registration[field] = entry.isIntersecting && entry.intersectionRect.width > 0 && entry.intersectionRect.height > 0
        registration.notify(registration.visible ? 0 : registration.near ? 1 : null)
      }
    }
    nearbyObserver = new IntersectionObserver((entries) => update(entries, 'near'), { rootMargin: '300px', threshold: [0, 0.001] })
    visibleObserver = new IntersectionObserver((entries) => update(entries, 'visible'), { threshold: [0, 0.001] })
  }
  registrations.set(element, { near: false, visible: false, notify })
  nearbyObserver.observe(element)
  visibleObserver!.observe(element)
  return () => {
    nearbyObserver?.unobserve(element)
    visibleObserver?.unobserve(element)
    registrations.delete(element)
    if (registrations.size === 0) {
      nearbyObserver?.disconnect()
      visibleObserver?.disconnect()
      nearbyObserver = visibleObserver = null
    }
  }
}
