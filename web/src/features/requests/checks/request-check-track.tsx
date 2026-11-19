import type { RepoParams } from '@/api/types'
import { Popover } from '@/components/ui/popover'
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '@/components/ui/tooltip'
import { cn } from '@/lib/utils'
import { ChevronDown } from 'lucide-react'
import { type RequestChecksSummary, requestListedChecks } from './request-check-rows'
import { RequestCheckWorkflows } from './request-check-workflows'

export function RequestCheckTrack({ params, summary }: { params: RepoParams; summary: RequestChecksSummary }) {
  const { status } = summary
  const label = status?.tone === 'success' ? 'Checks passed' : status?.text
  const labelClass = cn('shrink-0 font-medium', status?.tone === 'danger' && 'text-danger-strong')
  if (!summary.track.length) return label ? <span className={labelClass}>{label}</span> : null
  return (
    <Popover
      align="auto"
      className="max-h-[min(26rem,60vh)] w-[min(22rem,calc(100vw-2rem))] overflow-y-auto"
      label="CI jobs"
      renderPanel={() => <RequestCheckWorkflows params={params} rows={requestListedChecks(summary)} />}
      renderTrigger={(props) => (
        <button
          {...props}
          aria-label={label ? ['CI', label, summary.counts].filter(Boolean).join(' · ') : 'CI jobs'}
          className="request-check-toggle"
          type="button"
        >
          <TooltipProvider delayDuration={0}>
            <span className="request-check-track" data-few={summary.track.length <= 2 || undefined}>
              {summary.track.map((check) => (
                <Tooltip key={check.key}>
                  <TooltipTrigger asChild>
                    <span className="request-check-bar" data-tone={check.tone} />
                  </TooltipTrigger>
                  <TooltipContent className="request-check-tooltip">
                    <span className="font-medium">{check.name}</span> {check.reason ?? check.state}
                  </TooltipContent>
                </Tooltip>
              ))}
            </span>
          </TooltipProvider>
          <span className={labelClass}>{label ?? 'CI jobs'}</span>
          <ChevronDown aria-hidden="true" />
        </button>
      )}
    />
  )
}
