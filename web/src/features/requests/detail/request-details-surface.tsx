import { type ReactNode, type RefObject, useState } from 'react'
import { createPortal } from 'react-dom'
import { SlidersHorizontal } from 'lucide-react'
import { RequestSideDrawer } from './request-side-drawer'

export function RequestDetailsSurface({
  children,
  onOpenChange,
  open,
  returnFocus,
}: {
  children: ReactNode
  onOpenChange: (open: boolean) => void
  open: boolean
  returnFocus: RefObject<HTMLElement | null>
}) {
  const [drawerTarget, setDrawerTarget] = useState<HTMLDivElement | null>(null)
  return (
    <>
      <aside className="request-details-rail min-w-0 border-l border-border">
        {drawerTarget ? createPortal(children, drawerTarget) : children}
      </aside>
      <RequestSideDrawer
        description="Lifecycle, invitees, ratings and git state."
        icon={<SlidersHorizontal />}
        onOpenChange={onOpenChange}
        open={open}
        returnFocus={returnFocus}
        title="Details"
      >
        <div ref={setDrawerTarget} />
      </RequestSideDrawer>
    </>
  )
}
