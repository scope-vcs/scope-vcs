import type { RepoLiveState } from '@/api/types'
import { RepoLayoutProvider } from '@/features/repo-detail/repo-layout-context'
import { VisibilityLogSection } from '@/features/repo-detail/visibility-log-section'
import { RepositorySearchProvider } from '@/features/repo-search/repository-search-context'
import { createRoot } from 'react-dom/client'
import '../shared/styles.css'

const live = {
  repo: {
    id: 'repo', owner_handle: 'dev', name: 'demo',
    views: [
      { id: 'public', name: 'Public', includes: [], readers: 'anyone' as const },
      { id: 'private', name: 'Private', includes: 'all' as const, readers: 'assigned' as const },
    ],
    access: { actor: 'Owner', view: 'private' },
  },
} as RepoLiveState

createRoot(document.getElementById('root')!).render(
  <RepoLayoutProvider live={live} subscribe={() => () => {}}>
    <RepositorySearchProvider>
      <main className="mx-auto max-w-4xl px-4">
        <VisibilityLogSection params={{ owner: 'dev', repo: 'demo' }} view="private" />
      </main>
    </RepositorySearchProvider>
  </RepoLayoutProvider>,
)
