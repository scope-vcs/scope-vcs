import { useState } from 'react'
import { createRoot } from 'react-dom/client'
import { RepositoryHtmlPreviewProvider } from '@/components/repository-html-preview-store'
import { RepositoryHtmlRenderer } from '@/components/repository-html-renderer'
import { WorkspaceTabStrip } from '@/components/workspace-tab-strip'
import { toggleTheme } from '@/lib/use-theme-type'
import '@/styles.css'

function App() {
  const [mode, setMode] = useState<'preview' | 'source'>('preview')
  const [tabs, setTabs] = useState([{ id: 'one', label: 'One.html' }, { id: 'two', label: 'Two.html' }])
  return <RepositoryHtmlPreviewProvider>
    <button onClick={() => setMode(mode === 'preview' ? 'source' : 'preview')}>Toggle source</button>
    <button onClick={toggleTheme}>Toggle theme</button>
    <WorkspaceTabStrip activeId="one" ariaLabel="Fixture files" onActivate={() => {}}
      onClose={id => { setTabs(tabs.filter(tab => tab.id !== id)); return 'one' }}
      onEmptyFocus={() => {}} onPin={() => {}} previewId={null} tabSetId="fixture" tabs={tabs} />
    <RepositoryHtmlRenderer key="README.html:oid" identity={'README.html\0oid'} path="README.html"
      mode={mode} source="<h1>Test preview</h1>" />
  </RepositoryHtmlPreviewProvider>
}
createRoot(document.getElementById('root')!).render(<App />)
