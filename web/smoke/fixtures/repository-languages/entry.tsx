import { createRoot } from 'react-dom/client'
import { RepositoryLanguageBar } from '@/features/repo-detail/repository-language-bar'
import '../shared/styles.css'

const languages = [
  { name: 'Rust', color: '#DD3515', bytes: 8_415_546 },
  { name: 'TypeScript', color: '#2F74C0', bytes: 4_012_089 },
  { name: 'JavaScript', color: '#F0DC4E', bytes: 1_197_100 },
  { name: 'HTML', color: '#E96228', bytes: 146_502 },
  { name: 'SQL', color: '#FFBF1E', bytes: 139_732 },
  { name: 'Python', color: '#3472A6', bytes: 95_983 },
  { name: 'Shell', color: '#262E28', bytes: 75_129 },
  { name: 'CSS', color: '#AA88AA', bytes: 46_393 },
  { name: 'Docker', color: '#0B5EA2', bytes: 7_824 },
]
const search = new URL(location.href).searchParams
if (search.has('dark')) document.documentElement.classList.add('dark')

createRoot(document.getElementById('root')!).render(
  <main className="mx-auto min-h-dvh w-full max-w-(--app-rail) bg-background">
    <p className="flex min-h-14 items-center px-(--app-inset) text-xs text-muted-foreground">
      Version control with a view for each audience
    </p>
    <RepositoryLanguageBar languages={search.has('empty') ? [] : search.has('dim') ? languages.slice(6, 7) : languages} />
    <section aria-label="Repository README" className="px-(--app-inset) py-7">
      <h1 className="text-2xl font-semibold">Scope</h1>
    </section>
  </main>,
)
