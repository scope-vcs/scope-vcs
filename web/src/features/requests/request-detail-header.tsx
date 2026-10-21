import type { RequestSummaryResponse } from '@/api/types.generated'
import { RelativeTimestamp } from '@/components/timestamp'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Check, Copy } from 'lucide-react'
import { useEffect, useState, type ReactNode } from 'react'
import { toast } from 'sonner'
import { requestMergeabilityLabel, requestMergeabilityTone } from './request-labels'
import { useRequestWorkspace } from './request-workspace-context'
import { actorHandle } from './request-actor'

export function RequestDetailHeader({
  actions,
  backLink,
  request,
  viewName,
}: {
  actions?: ReactNode
  backLink: ReactNode
  request: RequestSummaryResponse
  viewName: string
}) {
  const workspace = useRequestWorkspace()
  const queueItem = workspace?.selected ?? null

  return (
    <header className="request-detail-header px-(--app-inset) pb-2 pt-3 min-[701px]:pt-6">
      <nav aria-label="Request navigation" className="mb-3 min-[701px]:hidden">{backLink}</nav>
      <h1 className="min-w-0 break-words text-[28px] font-semibold leading-[1.15] tracking-[-0.025em] sm:text-[30px]">
        {request.title}
      </h1>
      <div className="request-detail-header-secondary mt-4">
        <div className="request-detail-header-meta flex min-w-0 flex-wrap items-center gap-x-3 gap-y-2 text-[13px] leading-5 text-muted-foreground">
          <Badge variant={requestMergeabilityTone(request)}>
            {requestMergeabilityLabel(request)}
          </Badge>
          <span className="flex min-w-0 items-center gap-1">
            <span className="truncate font-mono text-xs">{request.name}</span>
            <CopyBranchButton branch={request.name} />
          </span>
          <span className="min-w-0 truncate">{viewName} view</span>
          {queueItem ? (
            <span className="flex flex-wrap items-center gap-x-1.5 gap-y-1">
              <span className="font-medium text-foreground">{actorHandle(queueItem.author)}</span>
              <span>opened</span>
              <RelativeTimestamp
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
        {actions ? (
          <div className="request-detail-header-actions flex min-w-0 items-center justify-end gap-2">
            {actions}
          </div>
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
