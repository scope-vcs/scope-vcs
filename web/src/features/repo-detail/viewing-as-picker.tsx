import type { ViewDefinition, ViewId } from '@/api/types.generated'
import { Button } from '../../components/ui/button'
import { Popover } from '../../components/ui/popover'
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '../../components/ui/tooltip'
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
  const current = options.find((option) => option.id === value)
  if (options.length < 2 || !current) return null
  return (
    <Popover
      align="auto"
      className="w-48 p-1"
      label="Viewing as"
      panel={(close) => (
        <>
          <p className="px-2 pt-1 pb-0.5 text-xs text-muted-foreground">Viewing as</p>
          {options.map((option) => (
            <button
              aria-current={option.id === value || undefined}
              className="flex w-full items-center gap-2 rounded-sm px-2 py-1.5 text-left text-xs hover:bg-muted focus-visible:bg-muted focus-visible:outline-2 focus-visible:outline-ring"
              key={option.id}
              onClick={() => {
                close()
                if (option.id !== value) onChange(option.id)
              }}
              type="button"
            >
              <Check aria-hidden="true" className={cn('size-3.5', option.id !== value && 'invisible')} />
              <span className="truncate">{option.name}</span>
            </button>
          ))}
        </>
      )}
      trigger={(props) => (
        <ViewingAsTooltip name={current.name}>
          <Button
            className={cn('max-w-48', compact && 'h-7 gap-1 px-2 text-xs')}
            size="sm"
            type="button"
            variant="secondary"
            {...props}
          >
            <Eye aria-hidden="true" className="size-3.5 text-muted-foreground" />
            <span className="truncate"><span className="sr-only">Viewing as </span>{current.name}</span>
            <ChevronDown
              aria-hidden="true"
              className={cn('size-3.5 text-muted-foreground transition-transform', props['aria-expanded'] && 'rotate-180')}
            />
          </Button>
        </ViewingAsTooltip>
      )}
    />
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
