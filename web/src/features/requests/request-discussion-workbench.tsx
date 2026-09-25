import type { RequestParams } from '@/api/types'
import type { RequestSummaryResponse } from '@/api/types.generated'
import { EmptyState } from '@/components/empty-state'
import { mainScrollContainer } from '@/components/main-content'
import { Button } from '@/components/ui/button'
import { CircleAlert, LoaderCircle, MessageSquare } from 'lucide-react'
import { useCallback, useEffect, useMemo, useState } from 'react'
import {
  readRequestDiscussionScroll,
  writeRequestDiscussionScroll,
} from './request-discussion-cache'
import { RequestDiscussionComposer } from './request-discussion-composer'
import type { RequestDiscussionActions } from './request-discussion-store'
import { useRequestDiscussionStore } from './request-discussion-store'
import { RequestDiscussionThread } from './request-discussion-thread'
import { RequestRevisionRow, useRequestRevisionPushes } from './request-revision-row'
import { requestTimelineItems } from './request-timeline-items'
import type {
  RequestActorSummary,
  RequestDiscussion,
  RequestDiscussionPage,
} from './request-discussion-types'
import type {
  RequestDiscussionThreadActions,
} from './use-request-discussion-replies'

export function RequestDiscussionWorkbench({
  actions,
  actor,
  canResolve,
  focusedDiscussionId,
  initialPage,
  params,
  permissions,
  repoId,
  request,
  threadActions,
}: {
  actions: RequestDiscussionActions
  actor: RequestActorSummary
  canResolve: (discussion: RequestDiscussion) => boolean
  focusedDiscussionId?: string
  initialPage: RequestDiscussionPage
  params: RequestParams
  permissions: {
    canOpenDiscussion: boolean
    canReply: boolean
    canWaitAfterReply: boolean
  }
  repoId: string
  request: RequestSummaryResponse
  threadActions: RequestDiscussionThreadActions
}) {
  const store = useRequestDiscussionStore({
    actions,
    actor,
    initialPage,
    params,
    repoId,
  })
  const pushes = useRequestRevisionPushes()
  const hasEarlierDiscussions = store.collection.nextCursor !== null
  const timeline = useMemo(
    () => requestTimelineItems(store.discussions, pushes, hasEarlierDiscussions),
    [hasEarlierDiscussions, pushes, store.discussions],
  )
  const [activeComposer, setActiveComposer] = useState<string | null>(null)
  const closeComposer = useCallback(() => setActiveComposer(null), [])

  useEffect(() => {
    const scrollContainer = mainScrollContainer()
    if (!scrollContainer) return
    scrollContainer.scrollTop = readRequestDiscussionScroll(store.cacheKey)
    return () => {
      writeRequestDiscussionScroll(store.cacheKey, scrollContainer.scrollTop)
    }
  }, [store.cacheKey])

  useEffect(() => {
    if (!focusedDiscussionId) return
    const frame = requestAnimationFrame(() => {
      document
        .querySelector(`#discussion-${CSS.escape(focusedDiscussionId)}`)
        ?.scrollIntoView({ block: 'start' })
    })
    return () => cancelAnimationFrame(frame)
  }, [focusedDiscussionId, store.cacheKey])

  const canStartDiscussion =
    permissions.canOpenDiscussion &&
    !['Closed', 'Merged'].includes(request.state)

  return (
    <section aria-label="Request discussion">
      {store.error ? (
        <div
          className="flex items-center gap-2 border-b border-border px-5 py-3 text-sm text-destructive lg:px-7"
          role="alert"
        >
          <CircleAlert className="size-4" />
          {store.error}
        </div>
      ) : null}

      {store.collection.nextCursor ? (
        <div className="border-b border-border px-5 py-4 text-center lg:px-7">
          <Button
            aria-busy={store.loadingMore}
            disabled={store.loadingMore}
            onClick={() => void store.loadMore()}
            size="sm"
            type="button"
            variant="secondary"
          >
            {store.loadingMore ? <LoaderCircle className="animate-spin" /> : null}
            Load earlier discussions
          </Button>
        </div>
      ) : null}

      {timeline.length > 0 ? (
        <div>
          {timeline.map((item) => item.kind === 'revision' ? (
            <RequestRevisionRow key={item.push.id} params={params} push={item.push} />
          ) : (
            <RequestDiscussionThread
              actions={threadActions}
              actor={actor}
              canReply={permissions.canReply}
              canResolve={canResolve(item.discussion)}
              canWaitAfterReply={permissions.canWaitAfterReply}
              composerOpen={activeComposer === item.discussion.id}
              discussion={item.discussion}
              key={`${store.cacheKey}\0${item.discussion.id}`}
              onExpandedChange={store.setExpanded}
              onMarkRead={store.markRead}
              onCloseComposer={closeComposer}
              onOpenComposer={() => setActiveComposer(item.discussion.id)}
              onPatch={store.patch}
              onRetryRoot={store.retry}
              onResolve={store.resolve}
              params={params}
            />
          ))}
        </div>
      ) : (
        <EmptyState
          description="Open one to ask a question or leave review notes."
          icon={<MessageSquare />}
          title="No discussions yet"
        />
      )}

      {canStartDiscussion ? (
        <div className="border-t border-border px-5 py-4 lg:px-7">
          <RequestDiscussionComposer onSubmit={store.create} />
        </div>
      ) : null}
    </section>
  )
}
