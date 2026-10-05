import { createRoot } from 'react-dom/client'
import { useState } from 'react'
import { Route } from '@/routes/account'
import { Viewer } from './clerk'
import { resetViewerState } from '@/lib/viewer-state'

const first = { viewerId: 'viewer-one', sessions: { sessions: [{ id: 'first', label: 'First CLI', created_at_unix: 1, last_used_at_unix: null, expires_at_unix: 2 }] } }
const second = { viewerId: 'viewer-one', sessions: { sessions: [{ id: 'second', label: 'Second CLI', created_at_unix: 1, last_used_at_unix: null, expires_at_unix: 2 }] } }

window.accountRouteResponse = first
const initial = await Route.loader()

function App() {
  const [loaded, setLoaded] = useState(initial)
  const [viewer, setViewer] = useState('viewer-one')
  const [mounted, setMounted] = useState(true)
  window.accountRouteLoaded = loaded
  window.revalidateAccount = async () => {
    window.accountRouteResponse = second
    setLoaded(await Route.loader())
  }
  window.startDelayedAccountLoad = () => {
    window.delayAccountResponse = true
    window.delayedAccountLoad = Route.loader().then(setLoaded)
  }
  window.removeAccountSession = (id: string) => {
    const current = window.accountRouteResponse
    window.accountRouteResponse = {
      ...current, sessions: { sessions: current.sessions.sessions.filter((session) => session.id !== id) },
    }
  }
  window.leaveAccount = () => setMounted(false)
  window.returnAccount = () => setMounted(true)
  window.switchAccount = async () => {
    resetViewerState()
    setViewer('viewer-two')
    window.accountRouteResponse = { viewerId: 'viewer-two', sessions: { sessions: [{ id: 'third', label: 'Third CLI', created_at_unix: 1, last_used_at_unix: null, expires_at_unix: 2 }] } }
    setLoaded(await Route.loader())
  }
  const AccountRoute = Route.component
  return <Viewer value={viewer}>{mounted ? <AccountRoute /> : <p>Other page</p>}</Viewer>
}

createRoot(document.getElementById('root')!).render(<App />)
