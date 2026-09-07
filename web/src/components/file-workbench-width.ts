export function clampFilePaneWidth(width: number) {
  return Math.min(360, Math.max(180, width))
}

export function filePaneKeyboardWidth(width: number, key: string): number | null {
  switch (key) {
    case 'ArrowLeft': return clampFilePaneWidth(width - 10)
    case 'ArrowRight': return clampFilePaneWidth(width + 10)
    case 'Home': return 180
    case 'End': return 360
    default: return null
  }
}
