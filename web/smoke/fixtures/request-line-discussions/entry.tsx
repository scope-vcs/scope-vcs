import { useState } from 'react'
import { createRoot } from 'react-dom/client'
import { createRootRoute, createRoute, createRouter, Outlet, RouterProvider } from '@tanstack/react-router'
import type { RepoLiveState } from '@/api/types'
import type { RequestDiscussionAnchor, RequestRevisionListResponse } from '@/api/types.generated'
import { RepoLayoutProvider } from '@/features/repo-detail/repo-layout-context'
import { RequestAttachmentProvider } from '@/features/requests/request-attachment-context'
import { RequestChangesWorkbench, type RequestChangesSearch } from '@/features/requests/request-changes-workbench'
import type { CreateDiscussionInput } from '@/features/requests/request-discussion-api'
import { discussion } from '@/features/requests/request-discussion-test-fixtures'
import './styles.css'

declare global {
  interface Window { diffHtml: string; createCalls: CreateDiscussionInput[] }
}

const base = 'a'.repeat(40)
const firstHead = 'b'.repeat(40)
const head = 'c'.repeat(40)
const path = '/src/lib.ts'
const file = { path, kind: 'Modified', old_oid: base, new_oid: head, old_mode: '100644', new_mode: '100644', label: 'private' }
const revision = (id: string, position: number, newHead: string) => ({
  base: { kind: 'main', review_base_oid: base }, base_version: 1, id, position, actor: null,
  old_head_oid: base, new_head_oid: newHead, inspection: 'Complete', created_at_unix: position,
  commits: [{ oid: newHead, parent_oids: [base], author: 'adam', message: `Revision ${position}`, change_count: 1, files_truncated: false, files: [file] }],
}) as RequestRevisionListResponse['revisions'][number]
const revisions = {
  revisions: [revision('revision-1', 1, firstHead), revision('revision-2', 2, head)],
  review_revision_id: 'revision-2',
  has_earlier_revisions: false,
  comparison: { revision_id: 'revision-2', mode: 'request', base_oid: base, head_oid: head, inspection: 'Complete', files: [file] },
} as RequestRevisionListResponse
const anchor = (overrides: Partial<RequestDiscussionAnchor>): RequestDiscussionAnchor => ({
  revision_id: 'revision-2', revision_position: 2, commit_oid: null, path, lines: null, latest: null, ...overrides,
})
const discussions = [
  discussion('carried', 4, {
    body_markdown: 'Carried forward review',
    anchor: anchor({
      revision_id: 'revision-1', revision_position: 1, lines: { side: 'new', start_line: 4, end_line: 5 },
      latest: { kind: 'mapped', revision_id: 'revision-2', start_line: 5, end_line: 6 },
    }),
  }),
  discussion('outdated', 3, {
    body_markdown: 'Outdated naming question',
    anchor: anchor({
      revision_id: 'revision-1', revision_position: 1, lines: { side: 'new', start_line: 3, end_line: 3 },
      latest: { kind: 'outdated', revision_id: 'revision-2' },
    }),
  }),
  discussion('hidden-lines', 2, {
    body_markdown: 'Lines outside the shown hunks',
    anchor: anchor({ lines: { side: 'new', start_line: 40, end_line: 40 } }),
  }),
  discussion('revision-level', 1, { body_markdown: 'Overall revision review', anchor: anchor({ path: null }) }),
]
const createCalls: CreateDiscussionInput[] = []
Object.assign(window, { createCalls })

const live = { repo: { id: 'repo', owner_handle: 'adam', name: 'demo', views: [{ id: 'private', name: 'Private', includes: 'all', readers: 'assigned' }], access: { actor: 'Owner', view: 'private' } } } as RepoLiveState
const attachmentActions = {
  list: async () => ({ attachments: [] }),
  limits: async () => ({ accepted_photo_media_types: [], accepted_video_media_types: [], max_attachments_per_content: 10 }),
}

function Changes() {
  const [search, setSearch] = useState<RequestChangesSearch>({ revision: 'revision-2', path })
  return (
    <RepoLayoutProvider live={live} subscribe={() => () => {}}>
      <RequestAttachmentProvider actions={attachmentActions as never} live={live} requestId="request" viewerId="adam">
        <RequestChangesWorkbench
          accessScope="fixture:private:adam"
          canOpenDiscussion
          createDiscussion={async (input) => {
            createCalls.push(input)
            const created = discussion(`created-${createCalls.length}`, 10 + createCalls.length, {
              body_markdown: input.body_markdown,
              client_discussion_id: input.client_discussion_id,
              anchor: anchor({ lines: input.anchor?.lines ?? null }),
            })
            discussions.unshift(created)
            return { discussion: created }
          }}
          initialDiscussionReferences={{ referenceKey: null, page: null }}
          loadDiff={async (input) => ({
            kind: 'Modified', new_mode: '100644', old_mode: '100644', path: input.path,
            presentation: { kind: 'html', html: window.diffHtml },
          })}
          loadDiscussions={async () => ({ discussions: [...discussions], next_cursor: null, snapshot_version: 10 + createCalls.length })}
          onSearchChange={setSearch}
          params={{ owner: 'adam', repo: 'demo', request_id: 'request' }}
          repoId="repo"
          revisions={revisions}
          search={search}
          view="private"
        />
      </RequestAttachmentProvider>
    </RepoLayoutProvider>
  )
}

const root = createRootRoute({ component: Outlet })
const request = createRoute({ getParentRoute: () => root, path: '/$owner/$repo/requests/$requestId', component: Changes })
const changes = createRoute({ getParentRoute: () => root, path: '/$owner/$repo/requests/$requestId/changes', component: Changes })
const router = createRouter({ routeTree: root.addChildren([request, changes]) })
createRoot(document.getElementById('root')!).render(<RouterProvider router={router} />)
