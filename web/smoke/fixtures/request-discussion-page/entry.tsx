import { useState } from 'react'
import { createRoot } from 'react-dom/client'
import {
  RequestDiscussionPageBoundary,
  type RequestDiscussionAvailability,
} from '@/features/requests/request-discussion-page-boundary'

function Composer() {
  const [draft, setDraft] = useState('')
  return <textarea aria-label="Start a new discussion" onChange={(event) => setDraft(event.target.value)} value={draft} />
}

function App() {
  const [available, setAvailable] = useState<RequestDiscussionAvailability>(() => new Promise((resolve) => {
    Object.assign(window, { resolveFirstLoad: () => resolve(true) })
  }))
  Object.assign(window, { refreshFromRetained: () => setAvailable(true) })
  return (
    <RequestDiscussionPageBoundary available={available} fallback={<p role="status">Loading request discussion</p>}>
      {(resolved) => resolved ? <Composer /> : <p>Discussion is unavailable</p>}
    </RequestDiscussionPageBoundary>
  )
}

createRoot(document.getElementById('root')!).render(<App />)
