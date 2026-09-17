import type { RequestSummaryResponse } from '@/api/types.generated'
import { RelativeTimestamp } from '@/components/timestamp'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Check, Copy } from 'lucide-react'
import { useEffect, useState, type ReactNode } from 'react'
import { toast } from 'sonner'
import {
  requestMergeabilityLabel,
  requestMergeabilityTone,
  requestStatusLabel,
  requestStatusTone,
} from './request-labels'
import { useRequestWorkspace } from './request-workspace-context'

/** Title plus the single meta line: state, branch, and who opened it. */
export function RequestDetailHeader({
  actions,
  request,
}: {
  actions?: ReactNode
  request: RequestSummaryResponse
}) {
  const workspace = useRequestWorkspace()
  const queueItem = workspace?.selected ?? null

  return (
    <header className="border-b border-border px-5 pb-4 pt-6 sm:px-6 lg:px-8">
      <div className="flex items-start justify-between gap-4">
        <h1 className="min-w-0 break-words text-[28px] font-medium leading-[1.1] tracking-[-0.03em] sm:text-[32px]">
          {request.title}
        </h1>
        {actions ? <div className="flex shrink-0 items-center gap-2 pt-1">{actions}</div> : null}
      </div>
      <div className="mt-3 flex flex-wrap items-center gap-x-3 gap-y-2 text-xs text-muted-foreground">
        <Badge stamp variant={requestStatusTone(request)}>
          {requestStatusLabel(request)}
        </Badge>
        {request.state === 'Open' ? (
          <Badge stamp variant={requestMergeabilityTone(request)}>
            {requestMergeabilityLabel(request)}
          </Badge>
        ) : null}
        <span className="flex min-w-0 items-center gap-1">
          <span className="truncate font-mono text-xs">{request.name}</span>
          <CopyBranchButton branch={request.name} />
        </span>
        {queueItem ? (
          <span className="flex flex-wrap items-center gap-x-1.5 gap-y-1">
            <span className="font-medium text-foreground">{queueItem.author.handle}</span>
            <span>opened</span>
            <RelativeTimestamp
              className="font-mono"
              value={request.submitted_at_unix ?? request.created_at_unix}
            />
            {queueItem.claimer ? (
              <>
                <span aria-hidden="true">·</span>
                <span>reviewing:</span>
                <span className="text-foreground">{queueItem.claimer.handle}</span>
              </>
            ) : null}
          </span>
        ) : null}
      </div>
    </header>
  )
}

function CopyBranchButton({ branch }: { branch: string }) {
  const [copied, setCopied] = useState(false)

  useEffect(() => {
    if (!copied) return
    const timeout = window.setTimeout(() => setCopied(false), 1200)
    return () => window.clearTimeout(timeout)
  }, [copied])

  async function copyBranch() {
    try {
      await navigator.clipboard.writeText(branch)
    } catch {
      toast.error('Copy failed')
      return
    }
    setCopied(true)
  }

  return (
    <Button
      aria-label="Copy branch name"
      onClick={() => void copyBranch()}
      size="icon-xs"
      title="Copy branch name"
      type="button"
      variant="ghost"
    >
      {copied ? <Check /> : <Copy />}
    </Button>
  )
}
