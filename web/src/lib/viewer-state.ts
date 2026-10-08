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
