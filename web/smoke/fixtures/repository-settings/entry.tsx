import { useState } from 'react'
import { createRoot } from 'react-dom/client'
import { createRootRoute, createRouter, RouterProvider } from '@tanstack/react-router'
import { RepoSettingsPage } from '@/features/repo-detail/repo-settings-page'
import { RepoLayoutProvider } from '@/features/repo-detail/repo-layout-context'
import { RepoCloneDropdown } from '@/features/repo-detail/repo-clone-dropdown'
import { CliSessionList } from '@/features/account/cli-session-list'
import { usePendingActions } from '@/lib/use-pending-actions'
import { useCachedResource } from '@/lib/use-cached-resource'
import { repoCollaborationResource, retainCollaborationResult } from '@/features/repo-detail/repo-collaboration-resource'
import type { RepoSummary, RepoLiveState, RepoMember, CliSession } from '@/api/types'
import { WorkspaceFixture } from './workspace'
import './styles.css'

const initial = {
  id: 'owner/demo', owner_handle: 'owner', name: 'demo', lifecycle_state: 'Ready',
  description: 'Original description', website_url: '', change_version: 0,
  access: { actor: 'Owner', can_manage_members: true },
} as RepoSummary
const members = ['alice', 'bob'].map((name) => ({
  user_id: name, handle: name, email: `${name}@example.com`,
  permissions: { can_push: false, can_change_file_visibility: false },
  created_at_unix: 1, updated_at_unix: 1,
})) as RepoMember[]
const sessions = ['session-a', 'session-b'].map((id) => ({ id, label: id, created_at_unix: 1, expires_at_unix: 100 })) as CliSession[]
const resolvers = new Map<string, () => void>()
const calls: unknown[] = []
Object.assign(window, { finishAction: (key: string) => resolvers.get(key)?.(), calls })
function hold(key: string) { return new Promise<void>((resolve) => resolvers.set(key, resolve)) }
const subscribe = () => () => {}
const settingsScope = 'fixture-owner'
repoCollaborationResource.write(settingsScope, { collaboration: { members, invites: [] } })
const loadSettings = () => new Promise<{ collaboration: null }>(() => {})

function App() {
  const [repo, setRepo] = useState(initial)
  const { pending, run } = usePendingActions()
  const settings = useCachedResource({ identity: settingsScope, resource: repoCollaborationResource, load: loadSettings, fallbackError: 'Settings unavailable' })
  return <main className="mx-auto max-w-5xl p-4">
    <div className="mb-6 flex flex-wrap gap-4">
      <button onClick={() => setRepo({ ...repo, description: 'Changed elsewhere', website_url: 'https://example.com' })}>Remote metadata update</button>
      <RepoCloneDropdown cloneRemoteUrl="https://example.com/owner/demo.git" repo={repo} />
      <button>After clone</button>
    </div>
    <RepoLayoutProvider live={{ repo } as RepoLiveState} subscribe={subscribe}>
      <RepoSettingsPage
        params={{ owner: 'owner', repo: 'demo' }} collaboration={settings.value?.collaboration ?? null}
        createInvite={async (input) => {
          calls.push(input)
          const invite = { id: 'new-invite', invited_email: input.email, permissions: input.permissions, state: 'Pending' as const, expires_at_unix: 100 }
          retainCollaborationResult(settingsScope, { type: 'inviteUpdated', invite })
          return { invite, invite_url: 'https://example.com/invites/new-token' }
        }}
        deleteInvite={async () => { throw new Error('unused') }}
        deleteMember={async () => { throw new Error('unused') }}
        deleteRepo={async () => { throw new Error('Deletion denied by fixture') }}
        updateMember={async (input) => {
          calls.push(input)
          await hold(input.member_user_id)
          const member = { ...members.find((member) => member.user_id === input.member_user_id)!, permissions: input.permissions }
          retainCollaborationResult(settingsScope, { type: 'memberUpdated', member })
          return member
        }}
        updateMetadata={async (metadata) => ({ ...repo, ...metadata })}
      />
    </RepoLayoutProvider>
    <h2>CLI sessions</h2>
    <button disabled={pending.has('grant')} onClick={() => void run('grant', () => hold('grant'))}>Create login command</button>
    <CliSessionList sessions={sessions} pending={pending} formatTime={String}
      revokeSession={(id) => void run(id, () => hold(id))} />
    <WorkspaceFixture />
  </main>
}
let loads = 0
const root = createRootRoute({ component: App, loader: () => ++loads === 1 ? {} : new Promise(() => {}) })
const router = createRouter({ routeTree: root })
createRoot(document.getElementById('root')!).render(<RouterProvider router={router} />)
