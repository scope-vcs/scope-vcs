import {
  clampFilePaneWidth,
  filePaneKeyboardWidth,
  FILE_PANE_MIN_WIDTH,
  FILE_PANE_MAX_WIDTH,
} from '../../components/file-workbench-width'

export const REQUEST_WORKSPACE_COLLAPSED_WIDTH = 54

const COLLAPSE_DRAG_THRESHOLD = 150
const REOPEN_DRAG_DISTANCE = 32

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
      width: clampFilePaneWidth(FILE_PANE_MIN_WIDTH + movement - REOPEN_DRAG_DISTANCE),
    }
  }

  const nextWidth = start.width + movement
  if (nextWidth < COLLAPSE_DRAG_THRESHOLD) {
    return { collapsed: true, width: start.width }
  }
  if (nextWidth < FILE_PANE_MIN_WIDTH) return null
  return { collapsed: false, width: clampFilePaneWidth(nextWidth) }
}

export function requestWorkspaceWidthFromKey(
  state: RequestWorkspaceWidthState,
  key: string,
): RequestWorkspaceWidthState | null {
  if (state.collapsed) {
    if (key === 'ArrowRight') return { collapsed: false, width: FILE_PANE_MIN_WIDTH }
    if (key === 'End') return { collapsed: false, width: FILE_PANE_MAX_WIDTH }
    return null
  }
  if (key === 'ArrowLeft' && state.width === FILE_PANE_MIN_WIDTH) {
    return { collapsed: true, width: state.width }
  }
  const width = filePaneKeyboardWidth(state.width, key)
  return width === null ? null : { collapsed: false, width }
}
