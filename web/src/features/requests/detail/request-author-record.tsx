import type { RequestAuthorRecordResponse } from '@/api/types.generated'
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '@/components/ui/tooltip'
import { GitPullRequest, Sprout, Star, Undo2 } from 'lucide-react'
import { useState } from 'react'
import { authorRecordTokens, type AuthorRecordToken } from './request-author-record-model'
import { RequestOutcomeIcon } from './request-outcome-icon'

const TOKEN_CLASS =
  '-mx-1 inline-flex cursor-default items-center gap-[3px] rounded-sm px-1 font-medium text-foreground hover:bg-accent focus-visible:outline-2 focus-visible:outline-ring data-[state=delayed-open]:bg-accent data-[state=instant-open]:bg-accent [&_svg]:size-3.5 [&_svg]:shrink-0'

export function RequestAuthorRecord({ detailed = false, record }: { detailed?: boolean; record: RequestAuthorRecordResponse }) {
  return (
    <TooltipProvider disableHoverableContent>
      <span className="mx-1 inline-flex items-center gap-2.5 self-center" data-author-record="">
        {authorRecordTokens(record, detailed).map((token) => (
          <AuthorRecordTokenMark detailed={detailed} key={token.kind} token={token} />
        ))}
      </span>
    </TooltipProvider>
  )
}

function AuthorRecordTokenMark({ detailed, token }: { detailed: boolean; token: AuthorRecordToken }) {
  const [open, setOpen] = useState(false)
  return (
    <Tooltip onOpenChange={setOpen} open={open}>
      <TooltipTrigger asChild onClick={(event) => {
        event.preventDefault()
        setOpen(true)
      }}>
        <button aria-label={token.tip} className={TOKEN_CLASS} data-kind={token.kind} tabIndex={detailed ? undefined : -1} type="button">
          <AuthorRecordIcon kind={token.kind} />
          {token.value}
          {detailed && token.ratingCount !== null && (
            <span className="text-[11px] font-normal text-muted-foreground">×{token.ratingCount}</span>
          )}
        </button>
      </TooltipTrigger>
      <TooltipContent>{token.tip}</TooltipContent>
    </Tooltip>
  )
}

function AuthorRecordIcon({ kind }: { kind: AuthorRecordToken['kind'] }) {
  switch (kind) {
    case 'merged':
    case 'closed':
      return <RequestOutcomeIcon outcome={kind} />
    case 'withdrawn':
      return <Undo2 aria-hidden="true" className="text-muted-foreground" />
    case 'open':
      return <GitPullRequest aria-hidden="true" className="text-muted-foreground" />
    case 'rating':
      return <Star aria-hidden="true" className="fill-current text-warning" />
    case 'first':
      return <Sprout aria-hidden="true" className="text-success" />
  }
}
