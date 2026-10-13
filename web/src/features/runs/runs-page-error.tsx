import { RouteErrorContent } from '@/components/route-error-page'
import { WorkbenchBar, WorkbenchPane } from '@/components/page-header'
import { PageErrorAlert } from '@/components/page-error-alert'
import { Button } from '@/components/ui/button'

export function RunsPageError({ error }: { error: unknown }) {
  return (
    <WorkbenchPane>
      <WorkbenchBar title="Runs" />
      <RouteErrorContent
        error={error}
        fallbackMessage="Unexpected runs error"
        title="Runs unavailable"
      />
    </WorkbenchPane>
  )
}

export function RunsRefreshAlert({ error, onRetry, title }: { error: string; onRetry: () => void; title: string }) {
  return (
    <div className="pt-5">
      <PageErrorAlert title={title}>
        <div className="flex flex-wrap items-center gap-3">
          <span>{error}</span>
          <Button onClick={onRetry} size="sm" variant="secondary">
            Retry now
          </Button>
        </div>
      </PageErrorAlert>
    </div>
  )
}
