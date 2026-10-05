const resets = new Set<() => void>()

export function onViewerChange(reset: () => void) {
  resets.add(reset)
}

export function resetViewerState() {
  for (const reset of resets) reset()
}
