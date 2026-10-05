import { createRoot, hydrateRoot } from 'react-dom/client'
import { RouterProvider } from '@tanstack/react-router'
import { createFixtureRouter, FixtureHydrationPage } from './app'
import './styles.css'

const handoff = window.__nativeRunHandoff
const router = handoff ? createFixtureRouter(handoff) : createFixtureRouter()
await router.load()
const app = handoff ? <FixtureHydrationPage handoff={handoff} router={router} /> : <RouterProvider router={router} />
const root = document.getElementById('root')!
if (handoff) hydrateRoot(root, app)
else createRoot(root).render(app)
