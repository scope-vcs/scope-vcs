import { renderToString } from 'react-dom/server'
import { createMemoryHistory } from '@tanstack/react-router'
import { repoResourceScope } from '@/features/repo-detail/repo-resource-scope'
import { resetViewerState } from '@/lib/viewer-state'
import { createFixtureRouter, FixtureHydrationPage } from './app'
import { loadRepoRunPage, loadDetail, loads, now } from './actions'

export async function renderFixture(url: string) {
  resetViewerState()
  Object.assign(loads, { history: 0, detail: 0, workflows: 0, logs: 0 })
  const scope = repoResourceScope({ id: 'repo-1', access: { actor: 'Owner' } }, 'adam')
  const handoff = url.split('?')[0].endsWith('/run-1')
    ? { scope, detail: await loadDetail(), now, loads: { ...loads } }
    : { scope, page: await loadRepoRunPage(), now, loads: { ...loads } }
  const router = createFixtureRouter(handoff, createMemoryHistory({ initialEntries: [url] }))
  await router.load()
  return { markup: renderToString(<FixtureHydrationPage handoff={handoff} router={router} />), handoff }
}
