import { createRoot } from 'react-dom/client'
import { ScopeLogo, ScopeMark } from '@/components/scope-logo'
import '../shared/styles.css'

function BrandFixture() {
  return (
    <main>
      {['light', 'dark'].map((theme) => (
        <section className={`${theme} bg-background p-4 text-foreground`} id={`brand-${theme}`} key={theme}>
          <header className="flex h-14 items-center gap-6"><ScopeLogo /><span>Repository</span></header>
          <div className="flex h-14 items-center gap-6">
            <ScopeMark className="size-4" />
            <ScopeMark />
            <ScopeMark className="size-6" />
          </div>
        </section>
      ))}
    </main>
  )
}

createRoot(document.getElementById('root')!).render(<BrandFixture />)
