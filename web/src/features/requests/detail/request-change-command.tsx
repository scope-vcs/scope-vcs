import type { RequestAskResponse } from '@/api/types.generated'
import { Button } from '@/components/ui/button'
import { Copy } from 'lucide-react'
import { toast } from 'sonner'

export function requestChangeCommand(ask: Pick<RequestAskResponse, 'handle' | 'suggested_branch_name'>) {
  return `scope request start ${ask.suggested_branch_name} --for ${ask.handle}`
}

export function RequestChangeCommand({ ask }: { ask: RequestAskResponse }) {
  const command = requestChangeCommand(ask)

  async function copy() {
    try {
      await navigator.clipboard.writeText(command)
    } catch {
      toast.error('Copy failed')
      return
    }
    toast.success('Copied')
  }

  return (
    <section
      aria-label="Add a change"
      className="flex flex-wrap items-center gap-x-3 gap-y-1.5 border-y border-border px-(--app-inset) py-2.5 text-xs text-muted-foreground"
    >
      <span>To add a change, run this in your checkout:</span>
      <span className="flex min-w-0 items-center gap-1 rounded-md bg-muted py-1 pr-1 pl-2">
        <code className="min-w-0 font-mono text-foreground [overflow-wrap:anywhere]">{command}</code>
        <Button aria-label="Copy command" onClick={() => void copy()} size="icon-xs" title="Copy command" type="button" variant="ghost">
          <Copy />
        </Button>
      </span>
    </section>
  )
}
