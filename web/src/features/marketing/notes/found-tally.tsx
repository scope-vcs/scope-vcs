import { cn } from '@/lib/utils'
import type { ReactElement, RefObject } from 'react'

/** Progress under the cursor while the mouse is held. The page is flooded with
 * the private view then, so the tally takes that view's theme. */
export function FoundTally({ found, inverseTheme, moreBelow, ref, total, visible }: { found: number; inverseTheme: string; moreBelow: boolean; ref: RefObject<HTMLDivElement | null>; total: number; visible: boolean }): ReactElement {
  const done = total > 0 && found === total
  return (
    <div aria-hidden className={cn(inverseTheme, 'found-tally pointer-events-none fixed left-0 top-0 z-[19] whitespace-nowrap font-mono text-xs text-muted-foreground', visible && 'is-visible')} ref={ref}>
      <span className="text-success-strong">{done ? `found all ${total}` : `found ${found} of ${total}`}</span>
      {!done && moreBelow && '. more further down'}
    </div>
  )
}
