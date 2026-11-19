import type { RepoLanguageResponse } from '@/api/types.generated'
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '@/components/ui/tooltip'
import { cn } from '@/lib/utils'
import { useState, type CSSProperties } from 'react'
import { languageSegments, languageSummary, type LanguageSegment } from './repository-languages'

export function RepositoryLanguageBar({ languages }: { languages: readonly RepoLanguageResponse[] | null }) {
  const [openIndex, setOpenIndex] = useState<number | null>(null)
  const segments = languageSegments(languages ?? [])
  if (segments.length === 0) return <div className="h-1 border-t border-border" />
  const summary = languageSummary(segments)

  return (
    <TooltipProvider delayDuration={100} disableHoverableContent>
      <div className="flex h-1 gap-0.5">
        {segments.map((segment, index) => (
          <Tooltip
            key={segment.name}
            onOpenChange={(open) => setOpenIndex((current) => open ? index : current === index ? null : current)}
            open={openIndex === index}
          >
            <TooltipTrigger
              aria-hidden={index > 0 || undefined}
              aria-label={index === 0 ? summary : undefined}
              className={cn(
                'relative h-full min-h-0! min-w-[3px] basis-0 cursor-default',
                'before:absolute before:inset-x-0 before:-inset-y-1.5 pointer-coarse:before:-top-2 pointer-coarse:before:-bottom-5',
                swatchClass(segment),
              )}
              onClick={(event) => {
                event.preventDefault()
                setOpenIndex(index)
              }}
              onPointerDown={(event) => event.preventDefault()}
              style={{ flexGrow: segment.share, ...swatchStyle(segment) }}
              tabIndex={index === 0 ? 0 : -1}
              type="button"
            />
            <TooltipContent
              className="border border-border-strong bg-popover px-3 py-2 text-popover-foreground shadow-[var(--shadow-pop)]"
              collisionPadding={12}
              side="bottom"
              sideOffset={8}
            >
              <ul className="min-w-44">
                {segments.map((row, rowIndex) => (
                  <li
                    className={cn(
                      'grid grid-cols-[8px_minmax(0,1fr)_auto] items-center gap-x-2 leading-[22px]',
                      rowIndex === index && 'font-semibold',
                    )}
                    key={row.name}
                  >
                    <span className={cn('size-2 rounded-xs', swatchClass(row))} style={swatchStyle(row)} />
                    <span>{row.name}</span>
                    <span className={cn('pl-3.5 tabular-nums', rowIndex !== index && 'text-muted-foreground')}>
                      {row.percent}
                    </span>
                  </li>
                ))}
              </ul>
            </TooltipContent>
          </Tooltip>
        ))}
      </div>
    </TooltipProvider>
  )
}

function swatchClass({ color }: LanguageSegment) {
  return color ? 'language-color' : 'bg-border-strong'
}

function swatchStyle({ color }: LanguageSegment): CSSProperties | undefined {
  return color ? { '--language-color': color } as CSSProperties : undefined
}
