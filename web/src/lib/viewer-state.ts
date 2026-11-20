const resets = new Set<() => void>()

export function onViewerChange(reset: () => void) {
  resets.add(reset)
}

export function resetViewerState() {
  for (const reset of resets) reset()
}

let currentViewerId: string | null | undefined
const viewerWaiters = new Set<(viewerId: string | null) => void>()

export function getCurrentViewerId() {
  return currentViewerId
}

export function whenViewerActive(boundMs: number): Promise<string | null | undefined> {
  const viewerId = currentViewerId
  if (viewerId !== undefined) return Promise.resolve(viewerId)
  return new Promise((resolve) => {
    const wake = (activeViewerId?: string | null) => {
      clearTimeout(timer)
      viewerWaiters.delete(wake)
      resolve(activeViewerId)
    }
    const timer = setTimeout(wake, boundMs)
    viewerWaiters.add(wake)
  })
}

export function activateViewer(viewerId: string | null) {
  const previous = currentViewerId
  currentViewerId = viewerId
  if (previous !== undefined && previous !== viewerId) resetViewerState()
  for (const wake of viewerWaiters) wake(viewerId)
  return previous
}

let currentSessionReady = false
const sessionReadyWaiters = new Set<() => void>()

export function sessionReady() {
  return currentSessionReady
}

export function markSessionReady() {
  currentSessionReady = true
  for (const wake of sessionReadyWaiters) wake()
}

export function whenSessionReady(boundMs: number) {
  if (currentSessionReady) return Promise.resolve()
  return new Promise<void>((resolve) => {
    const wake = () => {
      clearTimeout(timer)
      sessionReadyWaiters.delete(wake)
      resolve()
    }
    const timer = setTimeout(wake, boundMs)
    sessionReadyWaiters.add(wake)
  })
}
