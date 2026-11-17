import { Suspense, use, type ReactNode } from 'react'

export type RequestDiscussionAvailability = boolean | Promise<boolean>

type RenderDiscussion = (available: boolean) => ReactNode

export function RequestDiscussionPageBoundary({ available, children, fallback }: {
  available: RequestDiscussionAvailability
  children: RenderDiscussion
  fallback: ReactNode
}) {
  return (
    <Suspense fallback={fallback}>
      <ResolvedDiscussion available={available}>{children}</ResolvedDiscussion>
    </Suspense>
  )
}

function ResolvedDiscussion({ available, children }: {
  available: RequestDiscussionAvailability
  children: RenderDiscussion
}) {
  return children(available instanceof Promise ? use(available) : available)
}
