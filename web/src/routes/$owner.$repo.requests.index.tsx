import { createFileRoute } from '@tanstack/react-router'
import { GitPullRequest } from 'lucide-react'

export const Route = createFileRoute('/$owner/$repo/requests/')({ component: RequestSelection })

function RequestSelection() {
  return (
    <div className="flex min-h-80 flex-1 flex-col items-center justify-center px-6 text-center">
      <GitPullRequest aria-hidden="true" className="size-6 text-muted-foreground" />
      <h1 className="mt-4 text-lg font-medium">Select a request</h1>
      <p className="mt-2 text-sm text-muted-foreground">Review the discussion and changes here.</p>
    </div>
  )
}
