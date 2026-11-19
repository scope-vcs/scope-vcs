import { cn } from '@/lib/utils'
import type { RequestStateMark as Mark } from './request-labels'

const CHECK = 'm6 10.3 2.7 2.7L14 7.4'

const DISCS = {
  merged: { fill: 'fill-foreground', sign: CHECK },
  ready: { fill: 'fill-success', sign: CHECK },
  failed: { fill: 'fill-danger', sign: 'm7 7 6 6m0-6-6 6' },
  attention: { fill: 'fill-warning', sign: 'M10 5.6v5m0 3.6h.01' },
} as const

const RINGS = {
  waiting: { stroke: 'stroke-info-strong', dash: '3.6 3.47' },
  idle: { stroke: 'stroke-muted-foreground', dash: undefined },
} as const

export function RequestStateMark({ mark }: { mark: Mark }) {
  return (
    <svg aria-hidden="true" className="size-5 shrink-0" data-mark={mark} viewBox="0 0 20 20">
      {mark === 'waiting' || mark === 'idle' ? (
        <circle
          className={cn('fill-none stroke-2', RINGS[mark].stroke)}
          cx="10"
          cy="10"
          r="9"
          strokeDasharray={RINGS[mark].dash}
          strokeLinecap="round"
        />
      ) : (
        <>
          <circle className={DISCS[mark].fill} cx="10" cy="10" r="10" />
          <path
            className="fill-none stroke-background"
            d={DISCS[mark].sign}
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth="1.8"
          />
        </>
      )}
    </svg>
  )
}
