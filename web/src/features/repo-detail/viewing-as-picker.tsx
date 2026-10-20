import type { ViewDefinition, ViewId } from '@/api/types.generated'
import { Button } from '../../components/ui/button'
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '../../components/ui/tooltip'
import { ANCHORED_MENU_CLASS, useAnchoredMenu } from '../../components/ui/use-anchored-menu'
import { cn } from '../../lib/utils'
import { Check, ChevronDown, Eye } from 'lucide-react'
import type { ReactElement } from 'react'

export function ViewingAsPicker({
  compact = false,
  onChange,
  options,
  value,
}: {
  compact?: boolean
  onChange: (view: ViewId) => void
  options: readonly ViewDefinition[]
  value: ViewId
}) {
  const menu = useAnchoredMenu({ align: 'start' })
  const current = options.find((option) => option.id === value)
  if (options.length < 2 || !current) return null
  return (
    <>
      <ViewingAsTooltip name={current.name}>
        <Button
          {...menu.triggerProps}
          className={cn('max-w-48', compact && 'h-7 gap-1 px-2 text-xs')}
          size="sm"
          type="button"
          variant="secondary"
        >
          <Eye aria-hidden="true" className="size-3.5 text-muted-foreground" />
          <span className="truncate"><span className="sr-only">Viewing as </span>{current.name}</span>
          <ChevronDown
            aria-hidden="true"
            className={cn('size-3.5 text-muted-foreground transition-transform', menu.open && 'rotate-180')}
          />
        </Button>
      </ViewingAsTooltip>
      <div {...menu.menuProps} aria-label="Viewing as" className={cn(ANCHORED_MENU_CLASS, 'w-max min-w-48 max-w-[min(20rem,calc(100vw-16px))]')}>
        <p className="px-2 pt-1 pb-0.5 text-xs text-muted-foreground">Viewing as</p>
        {options.map((option) => (
          <button
            aria-checked={option.id === value}
            className="flex w-full items-start gap-2 rounded-sm px-2 py-1.5 text-left text-xs hover:bg-muted focus-visible:bg-muted focus-visible:outline-2 focus-visible:outline-ring"
            key={option.id}
            onClick={() => {
              menu.hide()
              if (option.id !== value) onChange(option.id)
            }}
            role="menuitemradio"
            type="button"
          >
            <Check aria-hidden="true" className={cn('mt-px size-3.5 shrink-0', option.id !== value && 'invisible')} />
            <span className="min-w-0 break-words">{option.name}</span>
          </button>
        ))}
      </div>
    </>
  )
}

export function ViewingAsStatus({ name }: { name: string }) {
  return (
    <ViewingAsTooltip name={name}>
      <span className="inline-flex items-center gap-1.5 text-sm text-muted-foreground">
        <Eye aria-hidden="true" className="size-3.5" />
        <span><span className="sr-only">Viewing as </span>{name}</span>
      </span>
    </ViewingAsTooltip>
  )
}

function ViewingAsTooltip({ children, name }: { children: ReactElement; name: string }) {
  return (
    <TooltipProvider>
      <Tooltip>
        <TooltipTrigger asChild>{children}</TooltipTrigger>
        <TooltipContent>Viewing as {name}</TooltipContent>
      </Tooltip>
    </TooltipProvider>
  )
}
