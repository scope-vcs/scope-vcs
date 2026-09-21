import { useCallback, useState } from 'react'
import { createRoot } from 'react-dom/client'
import { useRepoLiveRefresh } from '../../../src/features/repo-detail/repo-live-refresh'
import { loadRepoRouteState } from '../../../src/features/repo-detail/repo-route-recovery'
import { repoResourceScope } from '../../../src/features/repo-detail/repo-resource-scope'
import { useRequestQueue } from '../../../src/features/requests/use-request-queue'
import type { RepoLiveState } from '../../../src/api/types'
import type { LoadRequestQueuePage } from '../../../src/features/requests/request-queue-cache'

const repo = {
  id: 'owner/repo', owner_handle: 'owner', name: 'repo', description: null,
  website_url: null, git_remote_url: 'https://scope.test/repo.git',
  lifecycle_state: 'Ready', change_version: 7, open_request_count: 0,
  access: { actor: 'Owner', can_read_private_files: true, can_push: true,
    can_change_file_visibility: true, can_manage_members: true, can_delete_repo: true },
} satisfies RepoLiveState['repo']
const server = { ids: [] as string[], summaryReads: 0, queueReads: 0, connections: 0, streams: new Set<ReadableStreamDefaultController<Uint8Array>>() }
const live = (): RepoLiveState => ({ repo: { ...repo, open_request_count: server.ids.length }, event_stream_url: '/events', clerk_token_template: 'scope' })
const summary = () => loadRepoRouteState({ load: async () => { server.summaryReads++; return { live: live() } }, refresh: true, signal: new AbortController().signal })
const originalFetch = window.fetch
window.fetch = async (input, init) => {
  if (input !== '/events') return originalFetch(input, init)
  server.connections++
  return new Response(new ReadableStream({ start(controller) {
    server.streams.add(controller)
    controller.enqueue(new TextEncoder().encode(`event: repo-change\ndata: ${JSON.stringify({ repo_id: repo.id, incarnation_id: 'repo-i', kind: 'Connected', version: repo.change_version })}\n\n`))
    init?.signal?.addEventListener('abort', () => { if (server.streams.delete(controller)) controller.close() }, { once: true })
  } }), { headers: { 'content-type': 'text/event-stream' } })
}
const load: LoadRequestQueuePage = async section => {
  server.queueReads++
  return { requests: section === 'active' ? server.ids.map(id => ({ request: { id } })) as never[] : [], next_cursor: null, next_attention_at_unix: null }
}
function Queue({ current }: { current: RepoLiveState }) {
  const queue = useRequestQueue(repoResourceScope(current.repo, 'viewer'), String(current.repo.change_version), load)
  return <ul aria-label="Request queue">{queue.value?.pages.active.requests.map(item => <li key={item.request.id}>{item.request.id}</li>)}</ul>
}
function App({ initial }: { initial: Awaited<ReturnType<typeof summary>> }) {
  const [current, setCurrent] = useState(initial)
  const [visible, setVisible] = useState(true)
  const invalidate = useCallback(async () => { setCurrent(await summary()) }, [])
  useRepoLiveRefresh(current, invalidate, current.refreshId)
  Object.assign(window, { fixture: {
    server, refresh: invalidate,
    navigate: () => setVisible(value => !value),
    submit: (ids: string[]) => { server.ids = ids },
    interrupt: () => { for (const stream of server.streams) stream.close(); server.streams.clear() },
  } })
  return <main><h1>Requests <span data-count>{current.repo.open_request_count}</span></h1><button onClick={() => setVisible(value => !value)}>Navigate</button>{visible && <Queue current={current} />}</main>
}
summary().then(initial => createRoot(document.getElementById('root')!).render(<App initial={initial} />))
