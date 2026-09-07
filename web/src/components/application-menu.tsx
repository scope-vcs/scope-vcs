import { Link } from '@tanstack/react-router'
import { Ellipsis } from 'lucide-react'
import type { KeyboardEvent } from 'react'

export function ApplicationMenu() {
  return (
    <details
      className="relative"
      onBlur={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget)) event.currentTarget.open = false
      }}
    >
      <summary
        aria-label="Application menu"
        className="flex size-8 cursor-pointer list-none items-center justify-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring [&::-webkit-details-marker]:hidden"
        title="Application menu"
        onKeyDown={closeMenuOnEscape}
      >
        <Ellipsis aria-hidden="true" className="size-4" />
      </summary>
      <div className="absolute right-0 top-full z-50 mt-2 w-44 rounded-md border border-border bg-popover p-1 text-popover-foreground shadow-[var(--shadow-pop)]">
        <Link
          className="block rounded px-3 py-2 text-xs hover:bg-muted focus-visible:outline-2 focus-visible:outline-ring"
          onClick={(event) => {
            const menu = event.currentTarget.closest('details')
            if (menu) menu.open = false
          }}
          to="/licenses"
          onKeyDown={closeMenuOnEscape}
        >
          Scope licenses
        </Link>
      </div>
    </details>
  )
}

function closeMenuOnEscape(event: KeyboardEvent<HTMLElement>) {
  if (event.key !== 'Escape') return
  const menu = event.currentTarget.closest('details')
  if (!menu) return
  menu.open = false
  menu.querySelector('summary')?.focus()
}
