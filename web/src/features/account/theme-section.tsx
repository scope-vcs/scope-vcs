import { setThemePreference, THEME_PREFERENCES, useThemePreference, type ThemeType } from '@/lib/use-theme-type'
import { cn } from '@/lib/utils'
import { SettingsSection } from './settings-sections'

export function ThemeSection() {
  const preference = useThemePreference()
  return (
    <SettingsSection
      description="Saved in this browser. The switch in the page footer changes it too."
      id="theme"
      title="Theme"
    >
      <div aria-label="Theme" className="grid max-w-[480px] grid-cols-3 gap-3" role="group">
        {THEME_PREFERENCES.map(({ label, value }) => (
          <button
            aria-pressed={preference === value}
            className="group text-left focus-visible:outline-none"
            key={value}
            onClick={() => setThemePreference(value)}
            type="button"
          >
            <span className="flex h-16 overflow-hidden rounded-lg border border-border-strong group-hover:border-ring group-focus-visible:outline-2 group-focus-visible:outline-offset-2 group-focus-visible:outline-ring group-aria-pressed:shadow-[0_0_0_2px_var(--background),0_0_0_4px_var(--foreground)]">
              {value === 'system' ? (
                <>
                  <ThemePreview theme="light" />
                  <ThemePreview theme="dark" />
                </>
              ) : (
                <ThemePreview theme={value} />
              )}
            </span>
            <span className="mt-2 block text-sm">{label}</span>
          </button>
        ))}
      </div>
    </SettingsSection>
  )
}

function ThemePreview({ theme }: { theme: ThemeType }) {
  return (
    <span className={cn(theme, 'flex flex-1 flex-col gap-1.5 bg-background p-2.5')}>
      <span className="h-1.5 w-[70%] rounded-full bg-border" />
      <span className="h-1.5 w-[45%] rounded-full bg-border" />
    </span>
  )
}
