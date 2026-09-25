import { ApplicationTopbar } from '@/components/application-topbar'
import { AppShell } from '@/components/app-shell'
import { markdownComponents } from '@/components/markdown-components'
import { PageContent } from '@/components/page-header'
import { SafeMarkdown } from '@/components/safe-markdown'

/** Renders a policy authored in the repository's legal/ directory. */
export function LegalDocumentPage({ source }: { source: string }) {
  return (
    <AppShell header={() => <ApplicationTopbar />}>
      <PageContent>
        <article className="max-w-[72ch] text-[15px] leading-7 text-foreground">
          <SafeMarkdown components={markdownComponents('document')}>{source}</SafeMarkdown>
        </article>
      </PageContent>
    </AppShell>
  )
}
