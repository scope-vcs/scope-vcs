import { createRoot } from 'react-dom/client'
import { useState } from 'react'
import { RequestAttachmentProvider } from '@/features/requests/request-attachment-context'
import { RequestDiscussionMarkdown } from '@/features/requests/request-discussion-markdown'
import { RequestDescription } from '@/features/requests/request-description'
import { toggleTheme } from '@/lib/use-theme-type'
import { requestMermaidResource, resetRequestMermaidResource } from '@/features/requests/request-mermaid-resource'
import './styles.css'

const flowchart = '```mermaid\nflowchart LR\n  A[Request opened] --> B[Review changes]\n  B --> C[Merge request]\n```'
const sequence = '```mermaid\nsequenceDiagram\n  Alice->>Bob: Review this request\n  Bob-->>Alice: Looks good\n```'
const state = '```mermaid\nstateDiagram-v2\n  [*] --> Draft\n  Draft --> Ready\n  Ready --> [*]\n```'
const er = '```mermaid\nerDiagram\n  REQUEST ||--o{ DISCUSSION : contains\n  DISCUSSION ||--o{ REPLY : has\n```'
const actions = {
  list: async () => ({ attachments: [] }),
  limits: async () => ({}),
}

function App() {
  const [mode, setMode] = useState('ordinary')
  const [open, setOpen] = useState(true)
  const [revision, setRevision] = useState(0)
  const [viewer, setViewer] = useState('viewer')
  Object.assign(window, {
    mermaidCacheStats: () => requestMermaidResource.stats(),
    changeMermaidViewer: () => { resetRequestMermaidResource(); setViewer('other-viewer') },
  })
  const source = mode === 'ordinary' ? 'An ordinary request.\n\n```ts\nconst ready = true\n```' :
    mode === 'other' ? state :
    mode === 'invalid' ? '```mermaid\nnot a diagram !\n```' :
    mode === 'unsafe' ? '```mermaid\nflowchart LR\nA@{ "img": "//example.invalid/tracker.png" }\n```' :
    `${revision ? 'Updated explanation.\n\n' : ''}${flowchart}`
  return (
    <RequestAttachmentProvider actions={actions as never} live={{ repo: { id: 'repo', owner_handle: 'dev', name: 'demo', access: { actor: 'Public' } } } as never} requestId="request" viewerId={viewer}>
      <main className="mx-auto max-w-3xl px-5 py-8 text-foreground">
        <h1 className="mb-5 text-2xl">Request diagram review</h1>
        <nav className="mb-6 flex flex-wrap gap-4" aria-label="Fixture actions">
          <button onClick={() => setMode('diagrams')}>Show diagrams</button>
          <button onClick={() => setMode('other')}>Other diagrams</button>
          <button onClick={() => setMode('invalid')}>Invalid diagram</button>
          <button onClick={() => setMode('unsafe')}>Unsafe diagram</button>
          <button onClick={() => setOpen(!open)}>Toggle request</button>
          <button onClick={() => setRevision(revision + 1)}>Edit prose</button>
          <button onClick={toggleTheme}>Toggle theme</button>
        </nav>
        {open ? <>
          <section aria-label="Description"><RequestDescription canEdit={false} description={source} onSave={async () => false} /></section>
          {mode === 'other' ? <section aria-label="Discussion"><RequestDiscussionMarkdown source={er} /></section> : null}
          {mode === 'diagrams' ? <>
            <section aria-label="Discussion"><RequestDiscussionMarkdown source={flowchart} /></section>
            <div style={{ height: 1600 }} aria-hidden="true" />
            <section aria-label="Reply"><RequestDiscussionMarkdown source={sequence} /></section>
          </> : null}
        </> : <p>Request closed</p>}
      </main>
    </RequestAttachmentProvider>
  )
}
createRoot(document.getElementById('root')!).render(<App />)
