import type { RepoParams } from '@/api/types'
import type { LinkedRequestResponse } from '@/api/types.generated'
import { cn } from '@/lib/utils'
import { Link } from '@tanstack/react-router'

export const FACT_LINK_CLASS = 'rounded-sm underline decoration-border-strong underline-offset-[3px] hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring'

export function LinkedRequestLink({
  className,
  params,
  request,
}: {
  className?: string
  params: RepoParams
  request: Pick<LinkedRequestResponse, 'id' | 'title'>
}) {
  return (
    <Link
      className={cn(FACT_LINK_CLASS, className)}
      params={{ ...params, requestId: request.id }}
      search={(current) => ({ view: current.view })}
      title={request.title}
      to="/$owner/$repo/requests/$requestId"
    >
      {request.title}
    </Link>
  )
}
