import { useLayoutEffect, useState } from 'react'
import {
  closeWorkspaceTab,
  emptyWorkspaceTabState,
  openWorkspaceTab,
  pruneWorkspaceTabs,
} from './workspace-tab-model'

export function useWorkspaceTabs({
  activeId,
}: {
  activeId: string | null
}) {
  const [state, setState] = useState(() =>
    activeId ? openWorkspaceTab(emptyWorkspaceTabState, activeId, false) : emptyWorkspaceTabState,
  )

  useLayoutEffect(() => {
    if (!activeId) return
    setState((current) => openWorkspaceTab(current, activeId, false))
  }, [activeId])

  return {
    close(id: string, availableIds: ReadonlySet<string>) {
      const result = closeWorkspaceTab(
        pruneWorkspaceTabs(state, availableIds),
        activeId,
        id,
      )
      setState(result.state)
      return result
    },
    open(id: string, pinned: boolean) {
      setState((current) => openWorkspaceTab(current, id, pinned))
    },
    state,
  }
}
