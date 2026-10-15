import type { ReactNode } from 'react'
export function AppShell({ children }: { children: ReactNode }) { return <main>{children}</main> }
export function ApplicationTopbar() { return null }
export function CopyableCodeBlock() { return null }
export function PageContent({ children }: { children: ReactNode }) { return <>{children}</> }
export function PageErrorAlert({ children }: { children: ReactNode }) { return <>{children}</> }
export function Button({ children, ...props }: any) { return <button {...props}>{children}</button> }
export function ProfileSection() { return null }
export function ThemeSection() { return null }
export function SettingsPagePending() { return null }
export function SettingsLayout({ children }: { children: ReactNode }) { return <>{children}</> }
export function CliAccessSection({ action, children }: { action: ReactNode; children: ReactNode }) { return <>{action}{children}</> }
export function CliSessionList({ sessions, revokeSession }: { sessions: { id: string; label: string }[]; revokeSession: (id: string) => void }) {
  return <ul>{sessions.map((session) => <li key={session.id}>{session.label}<button onClick={() => revokeSession(session.id)}>Revoke {session.label}</button></li>)}</ul>
}
export function DeleteAccountSection() { return null }
export function AbsoluteTimestamp() { return null }
