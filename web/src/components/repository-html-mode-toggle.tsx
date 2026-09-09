import { Code, Eye } from 'lucide-react'
import type { RepositoryHtmlMode } from './repository-html-renderer'
import { ToggleGroup, ToggleGroupItem } from './ui/toggle-group'
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from './ui/tooltip'

// Tooltip triggers also set data-state, so selection styles follow aria-checked.
const modeItemClassName =
  'h-6 w-[26px] rounded-sm p-0 aria-checked:bg-foreground/20 aria-checked:text-foreground aria-checked:hover:bg-foreground/20 dark:aria-checked:hover:bg-foreground/20'

export function RepositoryHtmlModeToggle({
  mode,
  onSelect,
  path,
}: {
  mode: RepositoryHtmlMode
  onSelect: (mode: RepositoryHtmlMode) => void
  path: string
}) {
  return (
    <TooltipProvider>
      <ToggleGroup
        aria-label={`${path.replace(/^\/+/, '')} display mode`}
        className="rounded-md p-0.5"
        onValueChange={(value) => {
          if (value === 'preview' || value === 'source') onSelect(value)
        }}
        type="single"
        value={mode}
      >
        <Tooltip>
          <TooltipTrigger asChild>
            <ToggleGroupItem aria-label="Preview" className={modeItemClassName} value="preview">
              <Eye aria-hidden="true" className="size-3.5" />
            </ToggleGroupItem>
          </TooltipTrigger>
          <TooltipContent>Preview</TooltipContent>
        </Tooltip>
        <Tooltip>
          <TooltipTrigger asChild>
            <ToggleGroupItem aria-label="Source" className={modeItemClassName} value="source">
              <Code aria-hidden="true" className="size-3.5" />
            </ToggleGroupItem>
          </TooltipTrigger>
          <TooltipContent>Source</TooltipContent>
        </Tooltip>
      </ToggleGroup>
    </TooltipProvider>
  )
}
