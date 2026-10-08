import { Suspense, use, type ReactNode } from 'react'
import type { RequestDiscussionPage } from './request-discussion-types'

export type RequestDiscussionInitialPage = RequestDiscussionPage | null | Promise<RequestDiscussionPage | null>

type RenderDiscussionPage = (page: RequestDiscussionPage | null) => ReactNode

export function RequestDiscussionPageBoundary({ children, fallback, page }: {
  children: RenderDiscussionPage
  fallback: ReactNode
  page: RequestDiscussionInitialPage
}) {
  return (
    <Suspense fallback={fallback}>
      <ResolvedDiscussionPage page={page}>{children}</ResolvedDiscussionPage>
    </Suspense>
  )
}

function ResolvedDiscussionPage({ children, page }: {
  children: RenderDiscussionPage
  page: RequestDiscussionInitialPage
}) {
  return children(page instanceof Promise ? use(page) : page)
}
