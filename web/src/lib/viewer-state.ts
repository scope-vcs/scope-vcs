const resets = new Set<() => void>()

// Owners of viewer-scoped memory register here at module load, so a viewer
// change discards everything loaded so far without a hand-maintained list.
export function onViewerChange(reset: () => void) {
  resets.add(reset)
}

export function resetViewerState() {
  for (const reset of resets) reset()
}
