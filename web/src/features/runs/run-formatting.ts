export function formatDuration(seconds: number) {
  const safe = Math.max(0, Math.round(seconds))
  if (safe < 60) return `${safe}s`
  const minutes = Math.floor(safe / 60)
  if (minutes < 60) {
    const remaining = safe % 60
    return remaining === 0
      ? `${minutes}m`
      : `${minutes}m ${String(remaining).padStart(2, '0')}s`
  }
  const hours = Math.floor(minutes / 60)
  const remaining = minutes % 60
  return remaining === 0
    ? `${hours}h`
    : `${hours}h ${String(remaining).padStart(2, '0')}m`
}

export function elapsedDuration(
  start: number | null,
  end: number | null,
  nowUnix: number,
) {
  if (start === null) return null
  return formatDuration((end ?? nowUnix) - start)
}
