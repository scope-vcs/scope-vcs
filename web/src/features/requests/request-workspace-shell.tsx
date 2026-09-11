import { type CSSProperties, type ReactNode, useId, useState } from 'react'
import {
  REQUEST_WORKSPACE_COLLAPSED_WIDTH,
  requestWorkspaceWidthFromDrag,
  requestWorkspaceWidthFromKey,
} from './request-workspace-width'
import { PaneResizeHandle } from '@/components/pane-resize-handle'
import { FILE_PANE_MAX_WIDTH } from '@/components/file-workbench-width'
import './request-workspace-sidebar.css'

type RequestWorkspaceShellProps = {
  children: ReactNode
  collapsed: boolean
  detailOpenOnMobile: boolean
  onCollapsedChange: (collapsed: boolean) => void
  sidebar: ReactNode
}

export function RequestWorkspaceShell({
  children,
  collapsed,
  detailOpenOnMobile,
  onCollapsedChange,
  sidebar,
}: RequestWorkspaceShellProps) {
  const [width, setWidth] = useState(FILE_PANE_MAX_WIDTH)
  const sidebarId = useId()

  return (
    <div
      className="request-workspace-shell"
      data-collapsed={collapsed || undefined}
      data-detail-open={detailOpenOnMobile || undefined}
      style={{ '--request-workspace-sidebar-width': `${width}px` } as CSSProperties}
    >
      <div className="request-workspace-sidebar-container" id={sidebarId}>
        {sidebar}
      </div>
      <PaneResizeHandle
        className="request-workspace-resize-handle"
        controls={sidebarId}
        label="Requests sidebar width"
        max={FILE_PANE_MAX_WIDTH}
        min={REQUEST_WORKSPACE_COLLAPSED_WIDTH}
        onDrag={(distance) => {
          const next = requestWorkspaceWidthFromDrag(
            { startedCollapsed: collapsed, width, x: 0 },
            distance,
          )
          if (!next) return
          setWidth(next.width)
          onCollapsedChange(next.collapsed)
        }}
        onKey={(key) => {
          const next = requestWorkspaceWidthFromKey({ collapsed, width }, key)
          if (!next) return false
          setWidth(next.width)
          onCollapsedChange(next.collapsed)
          return true
        }}
        valueText={collapsed ? 'Collapsed' : `${width} pixels`}
        width={collapsed ? REQUEST_WORKSPACE_COLLAPSED_WIDTH : width}
      />
      <section className="request-workspace-detail">{children}</section>
    </div>
  )
}
