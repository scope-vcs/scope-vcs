import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '@/components/ui/tooltip'
import { GitPullRequest, Sprout, Star, Undo2 } from 'lucide-react'
import { useState } from 'react'
import type { AuthorRecordMark, AuthorRecordSummary, AuthorRecordToken } from './request-author-record-model'
import { RequestOutcomeIcon } from './request-outcome-icon'

const ICON_CLASS = '[&_svg]:size-3.5 [&_svg]:shrink-0'
const TOKEN_CLASS =
  `-mx-1 inline-flex cursor-default items-center gap-[3px] rounded-sm px-1 font-medium text-foreground hover:bg-accent focus-visible:outline-2 focus-visible:outline-ring data-[state=delayed-open]:bg-accent data-[state=instant-open]:bg-accent ${ICON_CLASS}`

export function RequestAuthorRecord({ tokens }: { tokens: AuthorRecordToken[] }) {
  if (!tokens.length) return null
  return (
    <TooltipProvider>
      <span className="mx-1 inline-flex items-center gap-2.5 self-center" data-author-record="">
        {tokens.map((token) => (
          <AuthorRecordTokenMark key={token.kind} token={token} />
        ))}
      </span>
    </TooltipProvider>
  )
}

function AuthorRecordTokenMark({ token }: { token: AuthorRecordToken }) {
  const [open, setOpen] = useState(false)
  return (
    <Tooltip onOpenChange={setOpen} open={open}>
      <TooltipTrigger asChild onClick={(event) => {
        event.preventDefault()
        setOpen(true)
      }}>
        <button aria-label={token.tip} className={TOKEN_CLASS} data-kind={token.kind} tabIndex={-1} type="button">
          <AuthorRecordIcon mark={token.kind} />
          {token.value}
        </button>
      </TooltipTrigger>
      <TooltipContent>{token.tip}</TooltipContent>
    </Tooltip>
  )
}

export function RequestAuthorRecordSummary({ handle, summary }: { handle: string; summary: AuthorRecordSummary }) {
  const [open, setOpen] = useState(false)
  return (
    <TooltipProvider>
      <Tooltip onOpenChange={setOpen} open={open}>
        <TooltipTrigger asChild onClick={(event) => {
          event.preventDefault()
          setOpen(true)
        }}>
          <button aria-label={summary.label} className={`${TOKEN_CLASS} gap-1 self-center`} data-author-record={summary.mark} type="button">
            <AuthorRecordIcon mark={summary.mark} />
            {summary.value}
          </button>
        </TooltipTrigger>
        <TooltipContent
          align="start"
          side="bottom"
          className={`grid gap-1 border border-border bg-popover px-3 py-2.5 leading-[18px] text-muted-foreground shadow-lg ${ICON_CLASS}`}
        >
          {summary.mark === 'first' ? null : (
            <span className="mb-0.5 border-b border-border pb-1.5">{handle} in this repository</span>
          )}
          {summary.lines.map((line) => (
            <span
              className={line.value === null ? 'flex items-center gap-2' : 'grid grid-cols-[14px_26px_1fr] items-center gap-x-2'}
              key={line.mark}
            >
              <AuthorRecordIcon mark={line.mark} />
              {line.value === null ? null : <span className="font-medium tabular-nums text-popover-foreground">{line.value}</span>}
              <span>{line.text}</span>
            </span>
          ))}
        </TooltipContent>
      </Tooltip>
    </TooltipProvider>
  )
}

function AuthorRecordIcon({ mark }: { mark: AuthorRecordMark }) {
  switch (mark) {
    case 'merged':
    case 'closed':
      return <RequestOutcomeIcon outcome={mark} />
    case 'withdrawn':
      return <Undo2 aria-hidden="true" className="text-muted-foreground" />
    case 'open':
      return <GitPullRequest aria-hidden="true" className="text-muted-foreground" />
    case 'rating':
      return <Star aria-hidden="true" className="fill-current text-warning" />
    case 'unrated':
      return <Star aria-hidden="true" className="text-muted-foreground" />
    case 'first':
      return <Sprout aria-hidden="true" className="text-success" />
  }
}
