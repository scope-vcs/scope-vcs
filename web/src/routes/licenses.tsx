/* Static license files are served by the web server, outside the client route tree. */
/* eslint-disable react-doctor/tanstack-start-no-anchor-element */
import { ApplicationTopbar } from '@/components/application-topbar'
import { AppShell } from '@/components/app-shell'
import { PageContent, PageHeader } from '@/components/page-header'
import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/licenses')({
  head: () => ({ meta: [{ title: 'Licenses · Scope' }] }),
  component: LicensesRoute,
})

const licenseLinkClass =
  'rounded-sm text-foreground underline decoration-muted-foreground/50 underline-offset-4 hover:decoration-foreground focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-4 focus-visible:outline-ring'

function LicensesRoute() {
  return (
    <AppShell header={() => <ApplicationTopbar />}>
      <PageContent>
        <div className="max-w-[68ch]">
          <PageHeader
            description="Licensing and attribution for Scope and the software it includes."
            title="Licenses"
          />

          <section aria-labelledby="scope-license" className="mt-10 border-t border-border pt-6">
            <h2 className="text-lg font-semibold" id="scope-license">Scope</h2>
            <p className="mt-3 text-[15px] leading-6 text-muted-foreground">
              Scope's own code is licensed under the Apache License, Version 2.0.
              Third-party components retain their own licenses.
            </p>
            <p className="mt-3 text-[15px] leading-6 text-muted-foreground">
              The bundled Pagent package is excluded from this grant; its separate
              license terms have not been supplied.
            </p>
            <ul className="mt-4 space-y-3 text-sm">
              <li><a className={licenseLinkClass} href="/LICENSE.txt">Apache License 2.0</a></li>
              <li><a className={licenseLinkClass} href="/NOTICE.txt">Scope attribution notice</a></li>
            </ul>
          </section>

          <section aria-labelledby="third-party-licenses" className="mt-8 border-t border-border pt-6">
            <h2 className="text-lg font-semibold" id="third-party-licenses">Third-party software</h2>
            <p className="mt-3 text-[15px] leading-6 text-muted-foreground">
              The following document contains the license texts and attribution notices
              for dependencies included with the Scope web application.
            </p>
            <p className="mt-4 text-sm">
              <a className={licenseLinkClass} href="/third-party-licenses.txt">Third-party licenses and notices</a>
            </p>
          </section>
        </div>
      </PageContent>
    </AppShell>
  )
}
