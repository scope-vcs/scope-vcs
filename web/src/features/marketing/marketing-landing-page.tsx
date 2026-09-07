import type { CliInstallCommands, CliPlatform } from '@/api/types'
import { ScopeLogo } from '@/components/scope-logo'
import { ThemeToggle } from '@/components/theme-toggle'
import { Link } from '@tanstack/react-router'
import { ArrowDown, ArrowUpRight } from 'lucide-react'
import type { ReactElement } from 'react'
import { ContributionFlow } from './contribution-flow'
import { MarketingCliOnboarding } from './marketing-cli-onboarding'
import { RepositoryProjection } from './repository-projection'
import './marketing-landing-page.css'
import './repository-projection.css'
import './contribution-flow.css'

const sourceUrl = 'https://scopevcs.com/adamblumoff/scope-vcs'

export function MarketingLandingPage({
  cliInstallCommands,
  initialCliPlatform,
}: {
  cliInstallCommands: CliInstallCommands
  initialCliPlatform: CliPlatform
}): ReactElement {
  return (
    <div className="marketing-page">
      <a className="skip" href="#main-content">Skip to content</a>
      <header className="topbar shell">
        <div className="brand">
          <Link aria-label="Scope home" to="/">
            <ScopeLogo className="logo" />
          </Link>
          <span className="alpha">pre-alpha</span>
        </div>
        <nav className="top-actions" aria-label="Account and appearance">
          <Link
            className="sign-in"
            params={{ _splat: '' }}
            search={{ redirect_url: '/' }}
            to="/sign-in/$"
          >
            Sign in
          </Link>
          <ThemeToggle />
        </nav>
      </header>
      <main className="shell" id="main-content" tabIndex={-1}>
        <section className="hero" aria-labelledby="hero-title">
          <div className="hero-copy">
            <h1 id="hero-title" className="enter">
              <span className="line">One repository.</span>{' '}
              <span className="line soft">You choose</span>{' '}
              <span className="line soft">what’s <span className="accent">public.</span></span>
            </h1>
            <p className="intro enter enter-two">
              Keep public and private code in one Git repository. Contributors clone
              only the files you share.
            </p>
            <div className="hero-actions enter enter-three">
              <a href="#install" className="primary">
                Install Scope<ArrowDown className="icon" aria-hidden />
              </a>
              <a href={sourceUrl} className="text-link">
                Browse Scope’s source<ArrowUpRight className="icon" aria-hidden />
              </a>
            </div>
          </div>
          <RepositoryProjection />
        </section>
        <section className="contributions" aria-labelledby="contribution-title">
          <div>
            <h2 id="contribution-title">Accept changes to<br />the code you share.</h2>
            <p className="section-copy">
              Review changes from the public clone and merge them into your repository.
              Your private code stays private.
            </p>
          </div>
          <ContributionFlow />
        </section>
        <section className="install" id="install" aria-labelledby="install-title">
          <div>
            <h2 id="install-title">Bring your repository.</h2>
            <p className="section-copy">Install the CLI, then connect an existing Git repository.</p>
          </div>
          <MarketingCliOnboarding
            commands={cliInstallCommands}
            initialPlatform={initialCliPlatform}
          />
        </section>
      </main>
      <footer className="footer shell">
        <Link aria-label="Scope home" to="/"><ScopeLogo className="logo" /></Link>
        <nav className="footer-right" aria-label="Project">
          <a href={sourceUrl}>Source code ↗</a>
          <Link to="/licenses">Licenses</Link>
        </nav>
      </footer>
    </div>
  )
}
