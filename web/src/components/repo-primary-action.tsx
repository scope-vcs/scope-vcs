import { Button } from '@/components/ui/button'
import { Link } from '@tanstack/react-router'
import { ArrowRight } from 'lucide-react'
import type { ComponentProps } from 'react'
import type { RepoSummaryResponse } from '@/api/types.generated'

type RepoPrimaryAction = {
  label: string
  to: '/$owner/$repo/settings'
}

function repoPrimaryAction(
  repo: RepoSummaryResponse,
  requireOwner = false,
): RepoPrimaryAction | null {
  if (requireOwner && repo.access.actor !== 'Owner') {
    return null
  }

  if (repo.lifecycle_state === 'AwaitingFirstPush' && repo.access.actor === 'Owner') {
    return { label: 'Clean up', to: '/$owner/$repo/settings' }
  }

  return null
}

export function RepoPrimaryActionButton({
  repo,
  requireOwner,
  variant = 'secondary',
}: {
  repo: RepoSummaryResponse
  requireOwner?: boolean
  variant?: ComponentProps<typeof Button>['variant']
}) {
  const action = repoPrimaryAction(repo, requireOwner)

  if (!action) {
    return null
  }

  return (
    <Button asChild size="sm" variant={variant}>
      <Link
        params={{ owner: repo.owner_handle, repo: repo.name }}
        to={action.to}
      >
        <ArrowRight className="size-3.5" />
        <span>{action.label}</span>
      </Link>
    </Button>
  )
}
