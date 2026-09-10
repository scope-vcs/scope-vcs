import { clampFilePaneWidth, filePaneKeyboardWidth } from '../../components/file-workbench-width'

export const REQUEST_WORKSPACE_COLLAPSED_WIDTH = 54

const COLLAPSE_DRAG_THRESHOLD = 150
const REOPEN_DRAG_DISTANCE = 32
const MIN_EXPANDED_WIDTH = 180
const MAX_EXPANDED_WIDTH = 360

export type RequestWorkspaceWidthState = {
  collapsed: boolean
  width: number
}

export type RequestWorkspaceWidthDrag = {
  startedCollapsed: boolean
  width: number
  x: number
}

export function requestWorkspaceWidthFromDrag(
  start: RequestWorkspaceWidthDrag,
  pointerX: number,
): RequestWorkspaceWidthState | null {
  const movement = pointerX - start.x
  if (start.startedCollapsed) {
    if (movement < REOPEN_DRAG_DISTANCE) return null
    return {
      collapsed: false,
      width: clampFilePaneWidth(MIN_EXPANDED_WIDTH + movement - REOPEN_DRAG_DISTANCE),
    }
  }

  const nextWidth = start.width + movement
  if (nextWidth < COLLAPSE_DRAG_THRESHOLD) {
    return { collapsed: true, width: start.width }
  }
  if (nextWidth < MIN_EXPANDED_WIDTH) return null
  return { collapsed: false, width: clampFilePaneWidth(nextWidth) }
}

export function requestWorkspaceWidthFromKey(
  state: RequestWorkspaceWidthState,
  key: string,
): RequestWorkspaceWidthState | null {
  if (state.collapsed) {
    if (key === 'ArrowRight') return { collapsed: false, width: MIN_EXPANDED_WIDTH }
    if (key === 'End') return { collapsed: false, width: MAX_EXPANDED_WIDTH }
    return null
  }
  if (key === 'ArrowLeft' && state.width === MIN_EXPANDED_WIDTH) {
    return { collapsed: true, width: state.width }
  }
  const width = filePaneKeyboardWidth(state.width, key)
  return width === null ? null : { collapsed: false, width }
}
