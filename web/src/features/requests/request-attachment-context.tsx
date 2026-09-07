import type {
  CreateRequestAttachmentMediaGrantResponse,
  RequestAttachmentListResponse,
  RequestAttachmentLimitsResponse,
  RequestAttachmentMediaTarget,
  RequestAttachmentResponse,
} from '@/api/types.generated'
import type { RepoChangeEvent } from '@/api/types.generated'
import { useRepoChangeSubscription } from '@/features/repo-detail/repo-layout-context'
import { repoResourceScope } from '@/features/repo-detail/repo-resource-scope'
import { useCachedResource } from '@/lib/use-cached-resource'
import { createContext, type ReactNode, use, useCallback, useEffect, useMemo } from 'react'
import type { RepoLiveState } from '@/api/types'
import {
  activateRequestAttachmentResourceScope,
  requestAttachmentResource,
  requestAttachmentResourceIdentity,
} from './request-attachment-resource'
import type {
  AttachmentUploadActions,
  AttachmentUploadParams,
} from './request-attachment-upload'
import { activateRequestAttachmentDraftScope } from './request-attachment-drafts'
import { activateRequestAttachmentMediaScope } from './request-attachment-media-resource'

export type RequestAttachmentActions = AttachmentUploadActions & {
  grant: (input: AttachmentUploadParams & {
    attachment_id: string
    target: RequestAttachmentMediaTarget
  }) => Promise<CreateRequestAttachmentMediaGrantResponse>
  list: (
    input: AttachmentUploadParams,
    signal?: AbortSignal,
  ) => Promise<RequestAttachmentListResponse>
  limits: (
    input: AttachmentUploadParams,
    signal?: AbortSignal,
  ) => Promise<RequestAttachmentLimitsResponse>
  retry: (input: AttachmentUploadParams & {
    attachment_id: string
    operation_id: string
  }) => Promise<RequestAttachmentResponse>
}

type RequestAttachmentContextValue = {
  accessScope: string
  actions: RequestAttachmentActions
  attachments: ReadonlyMap<string, RequestAttachmentResponse>
  attachmentsError: string | null
  attachmentsLoading: boolean
  limits: RequestAttachmentLimitsResponse | null
  isMaintainer: boolean
  params: AttachmentUploadParams
  repoId: string
  requestId: string
  viewerId: string
}

const RequestAttachmentContext = createContext<RequestAttachmentContextValue | null>(null)

export function RequestAttachmentProvider({
  actions,
  children,
  live,
  requestId,
  viewerId,
}: {
  actions: RequestAttachmentActions
  children: ReactNode
  live: RepoLiveState
  requestId: string
  viewerId: string
}) {
  const accessScope = repoResourceScope(live.repo, viewerId === 'anonymous' ? null : viewerId)
  const identity = requestAttachmentResourceIdentity(accessScope, requestId)
  useEffect(() => {
    activateRequestAttachmentDraftScope({ accessScope, repoId: live.repo.id, viewerId })
    activateRequestAttachmentMediaScope(accessScope)
    activateRequestAttachmentResourceScope(accessScope)
  }, [accessScope, live.repo.id, viewerId])
  const params = useMemo(() => ({
    owner: live.repo.owner_handle,
    repo: live.repo.name,
    request_id: requestId,
  }), [live.repo.name, live.repo.owner_handle, requestId])
  const load = useCallback(
    async (signal: AbortSignal) => {
      const [list, limits] = await Promise.all([
        actions.list(params, signal),
        actions.limits(params, signal),
      ])
      return { attachments: list.attachments, limits }
    },
    [actions, params],
  )
  const resource = useCachedResource({
    fallbackError: 'Attachments could not be loaded.',
    identity,
    load,
    resource: requestAttachmentResource,
  })

  const onRepoChange = useCallback((event: RepoChangeEvent) => {
    if (event.kind === 'Lagged') {
      requestAttachmentResource.invalidate(identity)
      return
    }
    if (
      typeof event.kind === 'object' &&
      (('RequestAttachmentChanged' in event.kind &&
        event.kind.RequestAttachmentChanged.request_id === requestId) ||
       ('RequestTimelineChanged' in event.kind &&
        event.kind.RequestTimelineChanged.request_id === requestId))
    ) {
      requestAttachmentResource.invalidate(identity)
    }
  }, [identity, requestId])
  useRepoChangeSubscription(onRepoChange)

  const attachments = useMemo(() => new Map(
    (resource.value?.attachments ?? []).map((attachment) => [attachment.id, attachment]),
  ), [resource.value])
  const value = useMemo<RequestAttachmentContextValue>(() => ({
    accessScope,
    actions,
    attachments,
    attachmentsError: resource.error,
    attachmentsLoading: resource.status === 'loading' || resource.refreshing,
    limits: resource.value?.limits ?? null,
    isMaintainer: live.repo.access.actor !== 'Public',
    params,
    repoId: live.repo.id,
    requestId,
    viewerId,
  }), [
    accessScope,
    actions,
    attachments,
    live.repo.id,
    live.repo.access.actor,
    params,
    requestId,
    resource.error,
    resource.refreshing,
    resource.status,
    resource.value?.limits,
    viewerId,
  ])
  return (
    <RequestAttachmentContext.Provider value={value}>
      {children}
    </RequestAttachmentContext.Provider>
  )
}

export function useRequestAttachments() {
  const context = use(RequestAttachmentContext)
  if (!context) throw new Error('request attachment context is unavailable')
  return context
}

export function refreshRequestAttachments(accessScope: string, requestId: string) {
  requestAttachmentResource.invalidate(
    requestAttachmentResourceIdentity(accessScope, requestId),
  )
}
