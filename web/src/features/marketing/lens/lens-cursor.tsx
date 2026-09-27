import type { ReactElement, RefObject } from 'react'

/** A dot that sits exactly on the pointer (the lens itself trails behind), and
 * grows into a ring over links and buttons. Hidden on touch screens. */
export function LensCursor({ ref }: { ref: RefObject<HTMLDivElement | null> }): ReactElement {
  return (
    <div aria-hidden className="lens-cursor pointer-events-none fixed left-0 top-0 z-20 pointer-coarse:hidden" ref={ref}>
      <i />
    </div>
  )
}
