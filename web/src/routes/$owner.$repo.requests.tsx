import { RepoContentError } from '@/components/repo-content-error'
import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/$owner/$repo/requests')({
  errorComponent: RepoContentError,
})
