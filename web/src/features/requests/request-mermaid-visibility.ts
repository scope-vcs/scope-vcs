type Priority = 0 | 1 | null
type Registration = { near: boolean; visible: boolean; notify: (priority: Priority) => void }
const groups = new Map<Element | null, ReturnType<typeof createObservers>>()

// A viewport root margin cannot extend past a nested scrolling ancestor.
// Share two observers per scroll container, including discussion drawers.
export function observeRequestMermaid(element: Element, notify: Registration['notify']) {
  const root = scrollRoot(element)
  let group = groups.get(root)
  if (!group) {
    group = createObservers(root)
    groups.set(root, group)
  }
  const { registrations, nearby, visible } = group
  registrations.set(element, { near: false, visible: false, notify })
  nearby.observe(element)
  visible.observe(element)
  return () => {
    nearby.unobserve(element)
    visible.unobserve(element)
    registrations.delete(element)
    if (registrations.size === 0) {
      nearby.disconnect()
      visible.disconnect()
      groups.delete(root)
    }
  }
}

function scrollRoot(element: Element) {
  for (let parent = element.parentElement; parent; parent = parent.parentElement) {
    if (/auto|scroll/.test(getComputedStyle(parent).overflowY)) return parent
  }
  return null
}

function createObservers(root: Element | null) {
  const registrations = new Map<Element, Registration>()
  const update = (entries: IntersectionObserverEntry[], field: 'near' | 'visible') => {
    for (const entry of entries) {
      const registration = registrations.get(entry.target)
      if (!registration) continue
      registration[field] = entry.isIntersecting && entry.intersectionRect.width > 0 && entry.intersectionRect.height > 0
      registration.notify(registration.visible ? 0 : registration.near ? 1 : null)
    }
  }
  const options = { root, threshold: [0, 0.001] }
  return {
    registrations,
    nearby: new IntersectionObserver((entries) => update(entries, 'near'), { ...options, rootMargin: '300px' }),
    visible: new IntersectionObserver((entries) => update(entries, 'visible'), options),
  }
}
