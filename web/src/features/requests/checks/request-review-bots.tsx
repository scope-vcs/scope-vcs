import { Popover } from '@/components/ui/popover'
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '@/components/ui/tooltip'
import { cn } from '@/lib/utils'
import { ChevronDown, GitPullRequest } from 'lucide-react'
import { RUN_TONE_TEXT_CLASS } from '../../runs/run-status-icon'
import {
  type RequestPullRequestView,
  type RequestReviewBotRow,
  requestReviewSummary,
} from './request-check-rows'

export function RequestReviewBots({
  bots,
  pullRequest,
}: {
  bots: RequestReviewBotRow[]
  pullRequest: RequestPullRequestView | null
}) {
  const summary = requestReviewSummary(bots)
  if (!summary) return null
  return (
    <Popover
      align="auto"
      className="w-[min(26rem,calc(100vw-2rem))] p-1"
      label="Review bots"
      renderPanel={() => (
        <>
          <ul aria-label="Review bots">
            {bots.map((bot) => (
              <li className="grid min-h-8 grid-cols-[20px_minmax(0,1fr)_auto] items-center gap-x-2 px-2 text-[13px]" key={bot.login}>
                <RequestReviewBotMark bot={bot} />
                <span className="flex min-w-0 flex-wrap items-baseline gap-x-2">
                  <span className="min-w-0 [overflow-wrap:anywhere]">{bot.name}</span>
                  {bot.required ? <span className="text-xs text-muted-foreground">required</span> : null}
                </span>
                <span className={cn('text-xs', RUN_TONE_TEXT_CLASS[bot.tone])}>{bot.label}</span>
              </li>
            ))}
          </ul>
          {pullRequest?.url ? (
            <a
              className="mt-1 grid min-h-8 grid-cols-[20px_minmax(0,1fr)] items-center gap-x-2 rounded border-t border-border px-2 pt-1 text-[13px] hover:bg-muted focus-visible:bg-muted focus-visible:outline-2 focus-visible:outline-ring"
              href={pullRequest.url}
              rel="noreferrer"
              target="_blank"
            >
              <GitPullRequest aria-hidden="true" className="size-3.5 justify-self-center text-muted-foreground" />
              {pullRequest.text}
            </a>
          ) : null}
        </>
      )}
      renderTrigger={(props) => (
        <button
          {...props}
          aria-label={`Review bots · ${summary.label}`}
          className="request-check-toggle"
          type="button"
        >
          <TooltipProvider delayDuration={0}>
            <span className="mr-0.5 flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1.5 py-0.5">
              {bots.map((bot) => (
                <Tooltip key={bot.login}>
                  <TooltipTrigger asChild>
                    <span><RequestReviewBotMark bot={bot} /></span>
                  </TooltipTrigger>
                  <TooltipContent>
                    <span className="font-medium">{bot.name}</span> {bot.required ? 'required · ' : ''}{bot.label}
                  </TooltipContent>
                </Tooltip>
              ))}
            </span>
          </TooltipProvider>
          <span className={cn('shrink-0', summary.state === 'commented' && 'text-warning-strong')}>{summary.label}</span>
          <ChevronDown aria-hidden="true" />
        </button>
      )}
    />
  )
}

function RequestReviewBotMark({ bot }: { bot: Pick<RequestReviewBotRow, 'name' | 'state'> }) {
  return (
    <span aria-hidden="true" className="request-review-bot" data-state={bot.state}>
      <span className="request-review-bot-face">{bot.name.charAt(0).toUpperCase()}</span>
      {bot.state === 'working' ? (
        <svg className="request-review-bot-arc" viewBox="0 0 22 22">
          <circle cx="11" cy="11" r="10" strokeDasharray="20 43" />
        </svg>
      ) : null}
      {bot.state === 'clear' ? (
        <span className="request-review-bot-badge">
          <svg viewBox="0 0 24 24">
            <circle className="fill-success" cx="12" cy="12" r="12" />
            <path className="fill-none stroke-white" d="m7 12.5 3.2 3.2L17 9" strokeLinecap="round" strokeLinejoin="round" strokeWidth="2.6" />
          </svg>
        </span>
      ) : null}
      {bot.state === 'commented' ? (
        <span className="request-review-bot-badge">
          <svg className="fill-warning" viewBox="0 0 24 24">
            <path d="M22 15a3 3 0 0 1-3 3H8l-6 5V5a3 3 0 0 1 3-3h14a3 3 0 0 1 3 3z" />
          </svg>
        </span>
      ) : null}
    </span>
  )
}
