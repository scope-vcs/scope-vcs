import { PageRail } from '@/components/page-header'
import {
  setThemePreference,
  THEME_PREFERENCES,
  useThemePreference,
  type ThemePreference,
} from '@/lib/use-theme-type'
import { cn } from '@/lib/utils'
import { Link } from '@tanstack/react-router'
import { Monitor, Moon, Sun } from 'lucide-react'

export const sourceUrl = 'https://scopevcs.com/adamblumoff/scope-vcs'

const THEME_ICONS: Record<ThemePreference, typeof Sun> = {
  dark: Moon,
  light: Sun,
  system: Monitor,
}

export function SiteFooter() {
  return (
    <footer className="border-t border-border" data-slot="site-footer">
      <PageRail className="flex min-h-14 flex-col items-start gap-3 py-4 text-xs text-muted-foreground sm:flex-row sm:flex-wrap sm:items-center sm:justify-between sm:gap-x-6 sm:py-3">
        <SiteLinks className="flex flex-wrap gap-x-5 gap-y-1 [&_a:hover]:text-foreground" label="Site" />
        <ThemeSwitch />
      </PageRail>
    </footer>
  )
}

export function SiteLinks({ className, label }: { className?: string; label?: string }) {
  return (
    <nav aria-label={label} className={className}>
      <a href={sourceUrl}>Source</a>
      <Link to="/privacy">Privacy</Link>
      <Link to="/terms">Terms</Link>
      <Link to="/licenses">Licenses</Link>
    </nav>
  )
}

export function ThemeSwitch({ className }: { className?: string }) {
  const preference = useThemePreference()
  return (
    <div
      aria-label="Theme"
      className={cn('inline-flex gap-px rounded-full border border-border p-0.5', className)}
      role="group"
    >
      {THEME_PREFERENCES.map(({ label, value }) => {
        const Icon = THEME_ICONS[value]
        return (
          <button
            aria-label={label}
            aria-pressed={preference === value}
            className="flex h-6 w-7 items-center justify-center rounded-full text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring aria-pressed:bg-muted aria-pressed:text-foreground"
            key={value}
            onClick={() => setThemePreference(value)}
            title={label}
            type="button"
          >
            <Icon aria-hidden className="size-3.5" />
          </button>
        )
      })}
    </div>
  )
}
