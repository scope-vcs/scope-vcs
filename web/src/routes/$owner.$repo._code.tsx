import { RepositoryCodePending } from '@/features/repo-detail/repository-code-pending'
import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/$owner/$repo/_code')({
  loader: async ({ parentMatchPromise }) => (await parentMatchPromise).loaderData,
  // This waits on the repository summary, so it pends whenever that refreshes.
  pendingComponent: RepositoryCodePending,
})
