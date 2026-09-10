import { type CSSProperties, type ReactNode, useId, useRef, useState } from 'react'
import {
  REQUEST_WORKSPACE_COLLAPSED_WIDTH,
  requestWorkspaceWidthFromDrag,
  requestWorkspaceWidthFromKey,
  type RequestWorkspaceWidthDrag,
} from './request-workspace-width'
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
  const [width, setWidth] = useState(360)
  const drag = useRef<RequestWorkspaceWidthDrag | null>(null)
  const sidebarId = useId()

  return (
    <div
      className="request-workspace-shell"
      data-collapsed={collapsed || undefined}
      data-detail-open={detailOpenOnMobile || undefined}
      style={{ '--request-workspace-sidebar-width': `${width}px` } as CSSProperties}
    >
      <div className="request-workspace-sidebar-container" id={sidebarId}>{sidebar}</div>
      <button
        aria-controls={sidebarId}
        aria-label="Requests sidebar width"
        aria-orientation="vertical"
        aria-valuemax={360}
        aria-valuemin={REQUEST_WORKSPACE_COLLAPSED_WIDTH}
        aria-valuenow={collapsed ? REQUEST_WORKSPACE_COLLAPSED_WIDTH : width}
        aria-valuetext={collapsed ? 'Collapsed' : `${width} pixels`}
        className="request-workspace-resize-handle"
        onKeyDown={(event) => {
          const next = requestWorkspaceWidthFromKey({ collapsed, width }, event.key)
          if (!next) return
          event.preventDefault()
          setWidth(next.width)
          onCollapsedChange(next.collapsed)
        }}
        onLostPointerCapture={() => {
          drag.current = null
        }}
        onPointerDown={(event) => {
          if (event.button !== 0) return
          event.preventDefault()
          event.currentTarget.focus()
          event.currentTarget.setPointerCapture(event.pointerId)
          drag.current = {
            startedCollapsed: collapsed,
            width: collapsed ? REQUEST_WORKSPACE_COLLAPSED_WIDTH : width,
            x: event.clientX,
          }
        }}
        onPointerMove={(event) => {
          const start = drag.current
          if (!start) return
          const next = requestWorkspaceWidthFromDrag(start, event.clientX)
          if (!next) return
          setWidth(next.width)
          onCollapsedChange(next.collapsed)
        }}
        onPointerUp={(event) => {
          drag.current = null
          if (event.currentTarget.hasPointerCapture(event.pointerId)) {
            event.currentTarget.releasePointerCapture(event.pointerId)
          }
        }}
        role="separator"
        tabIndex={0}
        type="button"
      />
      <section className="request-workspace-detail">{children}</section>
    </div>
  )
}

