import { Button } from '@/components/ui/button'
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/components/ui/tooltip'
import { cn } from '@/lib/utils'
import { Check, Copy } from 'lucide-react'
import { useEffect, useState } from 'react'
import { toast } from 'sonner'

export function CopyableCodeBlock({
  className,
  copyLabel = 'Copy',
  onCopy,
  value,
}: {
  className?: string
  copyLabel?: string
  onCopy?: () => void
  value: string
}) {
  const [copied, setCopied] = useState(false)

  useEffect(() => {
    if (!copied) {
      return
    }

    const timeout = window.setTimeout(() => setCopied(false), 1200)
    return () => window.clearTimeout(timeout)
  }, [copied])

  async function copyToClipboard() {
    try {
      await navigator.clipboard.writeText(value)
    } catch (error) {
      console.error('copy failed', error)
      toast.error('Copy failed')
      return
    }
    setCopied(true)
    toast.success('Copied')
    onCopy?.()
  }

  return (
    <div
      className={cn(
        'relative rounded-lg border border-border border-l-2 border-l-[var(--platinum)] bg-[var(--terminal-surface)] text-[var(--terminal-foreground)] shadow-[var(--shadow-card)]',
        className,
      )}
    >
      <pre className="overflow-x-auto whitespace-pre-wrap break-words px-3 py-2 pr-12 font-mono text-xs leading-5 [overflow-wrap:anywhere]">
        <code>{value}</code>
      </pre>
      <TooltipProvider>
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              aria-label={copied ? 'Copied' : copyLabel}
              className="absolute inset-y-0 right-2 my-auto border-white/15 bg-white/5 text-[#aeb4bf] hover:bg-white/10 hover:text-white"
              onClick={() => void copyToClipboard()}
              size="icon-sm"
              type="button"
              variant="secondary"
            >
              {copied ? <Check className="size-3.5" /> : <Copy className="size-3.5" />}
            </Button>
          </TooltipTrigger>
          <TooltipContent>{copied ? 'Copied' : copyLabel}</TooltipContent>
        </Tooltip>
      </TooltipProvider>
    </div>
  )
}
