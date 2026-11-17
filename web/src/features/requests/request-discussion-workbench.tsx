import type { RequestParams } from '@/api/types'
import type { RequestActorSummaryResponse, RequestSummaryResponse } from '@/api/types.generated'
import { EmptyState } from '@/components/empty-state'
import { mainScrollContainer } from '@/components/main-content'
import { Button } from '@/components/ui/button'
import { CircleAlert, LoaderCircle, MessageSquare } from 'lucide-react'
import { useCallback, useEffect, useState } from 'react'
import {
  readRequestDiscussionScroll,
  requestDiscussionCacheKey,
  writeRequestDiscussionScroll,
  type RequestDiscussionState,
} from './request-discussion-cache'
import { RequestDiscussionComposer } from './request-discussion-composer'
import type { RequestDiscussionActions } from './request-discussion-store'
import { useRequestDiscussionState, useRequestDiscussionStore } from './request-discussion-store'
import { RequestDiscussionThread } from './request-discussion-thread'
import type { RequestDiscussion } from './request-discussion-types'
import type {
  RequestDiscussionThreadActions,
} from './use-request-discussion-replies'

type RequestDiscussionWorkbenchProps = {
  actions: RequestDiscussionActions
  actor: RequestActorSummaryResponse
  canResolve: (discussion: RequestDiscussion) => boolean
  focusedDiscussionId?: string
  params: RequestParams
  permissions: {
    canOpenDiscussion: boolean
    canReply: boolean
    canWaitAfterReply: boolean
  }
  repoId: string
  viewerId: string
  request: RequestSummaryResponse
  threadActions: RequestDiscussionThreadActions
}

export function RequestDiscussionWorkbench(props: RequestDiscussionWorkbenchProps) {
  const cacheKey = requestDiscussionCacheKey({
    repoId: props.repoId,
    requestId: props.params.request_id,
    viewerId: props.viewerId,
  })
  const state = useRequestDiscussionState(cacheKey)
  return state ? <RetainedRequestDiscussion {...props} cacheKey={cacheKey} state={state} /> : null
}

function RetainedRequestDiscussion({
  actions,
  actor,
  cacheKey,
  canResolve,
  focusedDiscussionId,
  params,
  permissions,
  request,
  state,
  threadActions,
}: RequestDiscussionWorkbenchProps & { cacheKey: string; state: RequestDiscussionState }) {
  const store = useRequestDiscussionStore({
    actions,
    actor,
    cacheKey,
    params,
    requestHead: request.head_oid,
    state,
  })
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
          className="flex items-center gap-2 border-b border-border px-(--app-inset) py-3 text-sm text-destructive"
          role="alert"
        >
          <CircleAlert className="size-4" />
          {store.error}
        </div>
      ) : null}

      {store.collection.nextCursor ? (
        <div className="border-b border-border px-(--app-inset) py-4 text-center">
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

      {store.discussions.length > 0 ? (
        <div>
          {store.discussions.map((discussion) => (
            <RequestDiscussionThread
              actions={threadActions}
              actor={actor}
              canReply={permissions.canReply}
              canResolve={canResolve(discussion)}
              canWaitAfterReply={permissions.canWaitAfterReply}
              composerOpen={activeComposer === discussion.id}
              discussion={discussion}
              key={`${store.cacheKey}\0${discussion.id}`}
              onExpandedChange={store.setExpanded}
              onMarkRead={store.markRead}
              onCloseComposer={closeComposer}
              onOpenComposer={() => setActiveComposer(discussion.id)}
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
        <div className="border-t border-border px-(--app-inset) py-4">
          <RequestDiscussionComposer onSubmit={store.create} />
        </div>
      ) : null}
    </section>
  )
}
