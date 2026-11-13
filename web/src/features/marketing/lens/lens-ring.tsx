import { cn } from '@/lib/utils'
import type { ReactElement } from 'react'
import type { LensElements } from './use-lens'

export function LensRing({ elements, highlight, inverseTheme, label, showLabel }: { elements: LensElements; highlight: boolean; inverseTheme: string; label: string; showLabel: boolean }): ReactElement {
  const { edge, grip, label: labelRef, majorTicks, minorTicks, ring, ticks } = elements
  return (
    <div aria-hidden className="lens-ring pointer-events-none absolute left-0 top-0 z-[5] size-0 opacity-0" ref={ring}>
      <svg className="absolute left-0 top-0 overflow-visible">
        <circle className="fill-none stroke-border-strong" ref={edge} />
        <circle className="lens-grip" ref={grip} />
        <g ref={ticks}>
          <path className="stroke-muted-foreground" ref={minorTicks} strokeLinecap="round" />
          <path className="stroke-foreground" ref={majorTicks} strokeLinecap="round" />
        </g>
        <text className={cn('lens-label', inverseTheme, showLabel && 'is-shown', highlight && 'is-highlight')} ref={labelRef} textAnchor="middle">{label}</text>
      </svg>
    </div>
  )
}
