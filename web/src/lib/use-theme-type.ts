import { useSyncExternalStore } from 'react'

export type ThemeType = 'dark' | 'light'
export type ThemePreference = ThemeType | 'system'

export const THEME_PREFERENCES: { label: string; value: ThemePreference }[] = [
  { label: 'System', value: 'system' },
  { label: 'Light', value: 'light' },
  { label: 'Dark', value: 'dark' },
]

const THEME_STORAGE_KEY = 'scope-theme'
const THEME_CHANGE_EVENT = 'scope-theme-change'

export const themeBootScript = `(function(){var e=document.documentElement,q=matchMedia('(prefers-color-scheme: dark)');function apply(){var s=null;try{s=localStorage.getItem('${THEME_STORAGE_KEY}')}catch(_){}var dark=s==='dark'||(s!=='light'&&q.matches);e.classList.toggle('dark',dark);e.style.colorScheme=dark?'dark':'light'}apply();q.addEventListener('change',apply);addEventListener('storage',function(v){if(v.key==='${THEME_STORAGE_KEY}')apply()});addEventListener('${THEME_CHANGE_EVENT}',apply)})();`

export function useThemeType(): ThemeType {
  return useSyncExternalStore(subscribeToTheme, readBrowserTheme, readServerTheme)
}

export function useThemePreference(): ThemePreference {
  return useSyncExternalStore(subscribeToPreference, readStoredPreference, readServerPreference)
}

export function setThemePreference(preference: ThemePreference) {
  try {
    if (preference === 'system') localStorage.removeItem(THEME_STORAGE_KEY)
    else localStorage.setItem(THEME_STORAGE_KEY, preference)
  } catch {}
  window.dispatchEvent(new Event(THEME_CHANGE_EVENT))
}

function subscribeToTheme(onStoreChange: () => void) {
  if (typeof window === 'undefined') return () => undefined

  const observer = new MutationObserver(onStoreChange)
  observer.observe(document.documentElement, {
    attributeFilter: ['class'],
    attributes: true,
  })
  return () => observer.disconnect()
}

function subscribeToPreference(onStoreChange: () => void) {
  if (typeof window === 'undefined') return () => undefined

  window.addEventListener('storage', onStoreChange)
  window.addEventListener(THEME_CHANGE_EVENT, onStoreChange)
  return () => {
    window.removeEventListener('storage', onStoreChange)
    window.removeEventListener(THEME_CHANGE_EVENT, onStoreChange)
  }
}

function readBrowserTheme(): ThemeType {
  if (typeof document === 'undefined') return 'dark'
  return document.documentElement.classList.contains('dark') ? 'dark' : 'light'
}

function readServerTheme(): ThemeType {
  return 'dark'
}

function readStoredPreference(): ThemePreference {
  try {
    const stored = localStorage.getItem(THEME_STORAGE_KEY)
    return stored === 'dark' || stored === 'light' ? stored : 'system'
  } catch {
    return 'system'
  }
}

function readServerPreference(): ThemePreference {
  return 'system'
}
