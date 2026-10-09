const resets = new Set<() => void>()

export function onViewerChange(reset: () => void) {
  resets.add(reset)
}

export function resetViewerState() {
  for (const reset of resets) reset()
}

let currentViewerId: string | null | undefined

export function getCurrentViewerId() {
  return currentViewerId
}

export function activateViewer(viewerId: string | null) {
  const previous = currentViewerId
  currentViewerId = viewerId
  if (previous !== undefined && previous !== viewerId) resetViewerState()
  return previous
}

let currentSessionReady = false
const sessionReadyWaiters = new Set<() => void>()

export function sessionReady() {
  return currentSessionReady
}

export function markSessionReady() {
  currentSessionReady = true
  for (const wake of [...sessionReadyWaiters]) wake()
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
