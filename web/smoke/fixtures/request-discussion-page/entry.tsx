import { useState } from 'react'
import { createRoot } from 'react-dom/client'
import {
  RequestDiscussionPageBoundary,
  type RequestDiscussionInitialPage,
} from '@/features/requests/request-discussion-page-boundary'
import type { RequestDiscussionPage } from '@/features/requests/request-discussion-types'

const loaded: RequestDiscussionPage = { discussions: [], next_cursor: null, snapshot_version: 1 }

function Composer() {
  const [draft, setDraft] = useState('')
  return <textarea aria-label="Start a new discussion" onChange={(event) => setDraft(event.target.value)} value={draft} />
}

function App() {
  const [page, setPage] = useState<RequestDiscussionInitialPage>(() => new Promise((resolve) => {
    Object.assign(window, { resolveFirstLoad: () => resolve(loaded) })
  }))
  Object.assign(window, { refreshFromRetained: () => setPage({ ...loaded }) })
  return (
    <RequestDiscussionPageBoundary fallback={<p role="status">Loading request discussion</p>} page={page}>
      {(resolved) => resolved ? <Composer /> : <p>Discussion is unavailable</p>}
    </RequestDiscussionPageBoundary>
  )
}

createRoot(document.getElementById('root')!).render(<App />)
