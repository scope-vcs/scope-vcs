import { Button } from '@/components/ui/button'
import type { ReactNode } from 'react'

export function DetailsRow({ action, children, label }: { action?: ReactNode; children: ReactNode; label: string }) {
  return (
    <div className="grid min-h-7 grid-cols-[6.5rem_minmax(0,1fr)_auto] items-center gap-2">
      <span className="text-muted-foreground">{label}</span>
      <span className="min-w-0 truncate">{children}</span>
      {action ?? <span />}
    </div>
  )
}

export function DetailsToggle({ expanded, label, onToggle }: { expanded: boolean; label: string; onToggle: () => void }) {
  return (
    <Button aria-expanded={expanded} className="h-6 px-1.5 text-xs" onClick={onToggle} size="sm" type="button" variant="ghost">
      {expanded ? 'Hide' : label}
    </Button>
  )
}
