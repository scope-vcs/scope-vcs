import type { RequestDiffStatResponse, RequestSummaryResponse } from '@/api/types.generated'
import { DiffStatSummary } from '@/components/diff-stat'
import { diffStatWithChanges } from '@/lib/diff-stat'
import { RelativeTimestamp } from '@/components/timestamp'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Link } from '@tanstack/react-router'
import { Check, Copy } from 'lucide-react'
import { useEffect, useState, type ReactNode } from 'react'
import { toast } from 'sonner'
import { requestMergeabilityLabel, requestMergeabilityTone } from './request-labels'
import { useRequestWorkspace } from './request-workspace-context'
import { actorHandle } from './request-actor'

export function RequestDetailHeader({
  actions,
  backLink,
  diffStat,
  params,
  request,
  viewName,
}: {
  actions?: ReactNode
  backLink: ReactNode
  diffStat: RequestDiffStatResponse | null
  params: { owner: string; repo: string; requestId: string }
  request: RequestSummaryResponse
  viewName: string
}) {
  const workspace = useRequestWorkspace()
  const queueItem = workspace?.selected ?? null
  const changes = diffStatWithChanges(diffStat)

  return (
    <header className="request-detail-header px-(--app-inset) pb-2 pt-3 min-[701px]:pt-6">
      <nav aria-label="Request navigation" className="mb-3 min-[701px]:hidden">{backLink}</nav>
      <h1 className="min-w-0 break-words text-[28px] font-semibold leading-[1.15] tracking-[-0.025em] sm:text-[30px]">
        {request.title}
      </h1>
      <div className="request-detail-header-secondary mt-4">
        <div className="request-detail-header-meta min-w-0 text-xs leading-5 text-muted-foreground">
          <div className="request-detail-header-summary">
            <Badge className="h-7 border-0 bg-transparent p-0 text-[13px]" variant={requestMergeabilityTone(request)}>
              {requestMergeabilityLabel(request)}
            </Badge>
            <span className="request-detail-header-branch flex min-w-0 items-center gap-1">
              <span className="truncate font-mono text-xs" title={request.name}>{request.name}</span>
              <CopyBranchButton branch={request.name} />
            </span>
            {changes && (
              <Link
                className="request-detail-header-stat rounded hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring"
                params={params}
                search={{}}
                to="/$owner/$repo/requests/$requestId/changes"
              >
                <DiffStatSummary detail="exact" stat={changes} />
              </Link>
            )}
          </div>
          <div className="request-detail-header-author flex flex-wrap items-baseline gap-x-1.5">
            {queueItem ? (
              <span className="flex flex-wrap items-baseline gap-x-1.5">
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
            <span className="inline-flex items-baseline gap-1.5 whitespace-nowrap">
              {queueItem ? <span aria-hidden="true">·</span> : null}
              <span>{viewName} view</span>
            </span>
          </div>
        </div>
        {actions ? (
          <div className="request-detail-header-actions">
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
