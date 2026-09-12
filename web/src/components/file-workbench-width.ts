export const FILE_PANE_MIN_WIDTH = 180
export const FILE_PANE_MAX_WIDTH = 360

export function clampFilePaneWidth(width: number) {
  return Math.min(FILE_PANE_MAX_WIDTH, Math.max(FILE_PANE_MIN_WIDTH, width))
}

export function filePaneKeyboardWidth(width: number, key: string): number | null {
  switch (key) {
    case 'ArrowLeft':
      return clampFilePaneWidth(width - 10)
    case 'ArrowRight':
      return clampFilePaneWidth(width + 10)
    case 'Home':
      return FILE_PANE_MIN_WIDTH
    case 'End':
      return FILE_PANE_MAX_WIDTH
    default:
      return null
  }
}
