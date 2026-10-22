import type { RepoLiveState } from '@/api/types'
import type { CommitFileResponse, FileChangeKind, HistoryEntryDetailResponse } from '@/api/types.generated'
import { RepoLayoutProvider } from '@/features/repo-detail/repo-layout-context'
import { useChangedFileNavigation } from '@/features/history/changed-files-workbench'
import type { HistoryVisibilityChange } from '@/features/history/changed-paths-model'
import { HistoryEntryChanges } from '@/features/history/history-entry-changes'
import { useState } from 'react'
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

const KINDS: FileChangeKind[] = ['Modified', 'Added', 'Modified', 'Deleted', 'Modified', 'Added']
const FOLDERS: [string, number][] = [
  ['api/src/http', 20], ['api/src/git', 23], ['bench', 14], ['cli/src/request', 33],
  ['crates/scope-domain/src', 60], ['crates/scope-postgres/src/db', 38], ['docs', 12],
  ['web/src/features/history', 70], ['web/src/features/requests', 61], ['worker', 21],
]

function file(path: string, kind: FileChangeKind, label = 'private'): CommitFileResponse {
  return { path, kind, old_mode: '100644', new_mode: '100644', old_oid: 'a', new_oid: 'b', label }
}

const files = FOLDERS.flatMap(([folder, count]) =>
  Array.from({ length: count }, (_, index) => file(`/${folder}/file_${String(index).padStart(2, '0')}.rs`, KINDS[index % KINDS.length])))
files.push(file('/README.md', 'Modified', 'public'))

const changes: HistoryVisibilityChange[] = files
  .filter((_, index) => index % 5 !== 1 && index < 178)
  .map((entry) => ({
    id: `set:${entry.path}`, path: entry.path, old_label: 'private', new_label: 'public',
    file: { ...entry, kind: 'Added', old_oid: null, label: 'public' },
  }))
changes.push(
  { id: 'set2:/api/src/http/file_00.rs', path: '/api/src/http/file_00.rs', old_label: 'public', new_label: 'private', file: null },
  { id: 'set:/docs/policy.md', path: '/docs/policy.md', old_label: 'private', new_label: 'public', file: null },
)

const detail = { view: 'private', files, visibility_changes: changes } as unknown as HistoryEntryDetailResponse

function App() {
  const [selection, setSelection] = useState<{ path: string | null; visibilityId: string | null }>({ path: null, visibilityId: null })
  Object.assign(window, { selection })
  const navigation = useChangedFileNavigation(() => setSelection({ path: null, visibilityId: null }), true)
  const label = selection.visibilityId ? `visibility preview ${selection.visibilityId}` : `code diff ${selection.path}`
  return (
    <RepoLayoutProvider live={live} subscribe={() => () => {}}>
      <main className="mx-auto max-w-6xl">
        <HistoryEntryChanges
          detail={detail}
          diffIdentity={label}
          diffScrollTop={0}
          fileDiffState={selection.path
            ? {
                status: 'loaded',
                error: null,
                diff: { kind: 'Modified', old_mode: null, new_mode: null, path: selection.path, presentation: { kind: 'html', html: `<pre>${label}</pre>` } },
              }
            : { status: 'idle', error: null, diff: null }}
          navigation={navigation}
          onCloseDiff={navigation.closeDiff}
          onDiffScroll={() => {}}
          onSelectFile={(selected) => setSelection({ path: selected.path, visibilityId: null })}
          onSelectVisibility={(change) => setSelection({ path: change.path, visibilityId: change.id })}
          selectedFilePath={selection.path}
          selectedVisibilityId={selection.visibilityId}
        />
      </main>
    </RepoLayoutProvider>
  )
}

createRoot(document.getElementById('root')!).render(<App />)
