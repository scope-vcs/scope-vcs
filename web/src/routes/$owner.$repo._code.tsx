import { RepositoryCodePending } from '@/features/repo-detail/repository-code-pending'
import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/$owner/$repo/_code')({
  pendingComponent: RepositoryCodePending,
})
