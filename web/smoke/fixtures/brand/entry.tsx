import { createRoot } from 'react-dom/client'
import { ScopeLogo, ScopeMark } from '@/components/scope-logo'
import '../shared/styles.css'

function BrandFixture() {
  return (
    <main>
      <section className="bg-background p-4 text-foreground" id="brand-light">
        <header className="flex h-14 items-center gap-6"><ScopeLogo /><span>Repository</span></header>
        <div className="flex h-14 items-center gap-6">
          <ScopeMark className="size-4" />
          <ScopeMark />
          <ScopeMark className="size-6" />
        </div>
      </section>
      <section className="dark bg-background p-4 text-foreground" id="brand-dark">
        <header className="flex h-14 items-center gap-6"><ScopeLogo /><span>Repository</span></header>
        <div className="flex h-14 items-center gap-6">
          <ScopeMark className="size-4" />
          <ScopeMark />
          <ScopeMark className="size-6" />
        </div>
      </section>
    </main>
  )
}

createRoot(document.getElementById('root')!).render(<BrandFixture />)
