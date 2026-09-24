import { cn } from '@/lib/utils'
import type { ReactElement } from 'react'
import type { LensElements } from './use-lens'

/** The focus ring around the lens. Its size, ticks and position are written by
 * `useLens` every frame; React only sets the label, which sits inside the lens
 * and so takes the private view's theme. */
export function LensRing({ elements, highlight, inverseTheme, label, showLabel }: { elements: LensElements; highlight: boolean; inverseTheme: string; label: string; showLabel: boolean }): ReactElement {
  return (
    <div aria-hidden className="lens-ring pointer-events-none absolute left-0 top-0 z-[5] size-0 opacity-0" ref={elements.ring}>
      <svg className="absolute left-0 top-0 overflow-visible">
        <circle className="fill-none stroke-border-strong" ref={elements.edge} />
        <g ref={elements.ticks}>
          <path className="stroke-muted-foreground" ref={elements.minorTicks} strokeLinecap="round" />
          <path className="stroke-foreground" ref={elements.majorTicks} strokeLinecap="round" />
        </g>
        <text className={cn('lens-label', inverseTheme, showLabel && 'is-shown', highlight && 'is-highlight')} ref={elements.label} textAnchor="middle">{label}</text>
      </svg>
      <div className="lens-grab absolute hidden rounded-full pointer-coarse:pointer-events-auto pointer-coarse:block" ref={elements.grab} />
    </div>
  )
}
