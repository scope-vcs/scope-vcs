import type { RepoParams } from '@/api/types'
import type { RepoSummaryResponse } from '@/api/types.generated'
import {
  ApplicationTopbar,
  type TopbarItem,
} from '@/components/application-topbar'
import { AppShell } from '@/components/app-shell'
import {
  activeRepoSection,
  repoSectionsForActor,
} from '@/components/repo-section-model'
import { repoResourceScope } from '@/features/repo-detail/repo-resource-scope'
import { useRequestReturnLocation } from '@/features/requests/use-request-return-location'
import { UserButton, useAuth } from '@clerk/tanstack-react-start'
import { Link, useLocation, useRouter } from '@tanstack/react-router'
import type { ReactNode } from 'react'

/**
 * Repository chrome. While the repository loads, `repo` is null and the shell
 * shows the sections every visitor has, so the topbar keeps its size and tabs.
 */
export function RepoShell({
  children,
  params,
  repo,
}: {
  children: ReactNode
  params: RepoParams
  repo: RepoSummaryResponse | null
}) {
  const router = useRouter()
  const { isLoaded, userId } = useAuth()
  const requestLocation = useRequestReturnLocation(
    isLoaded && repo ? repoResourceScope(repo, userId ?? null) : null,
    router.buildLocation({ params, to: '/$owner/$repo/requests' }).pathname,
  )
  const pathname = useLocation({ select: (location) => location.pathname })
  const sections = repoSectionsForActor(repo?.access.actor ?? 'Public')
  const openRequestCount = repo?.open_request_count ?? 0
  const active = activeRepoSection((to) => {
    const target = router.buildLocation({ params, to }).pathname
    return pathname === target || pathname.startsWith(`${target}/`)
  })
  const returnToRequest = active === 'requests' ? null : requestLocation
  const items = sections.map<TopbarItem>(
    (section) => ({
      active: active === section.key,
      label: section.label,
      node: (
        <>
          <Link
            activeOptions={{ exact: section.key === 'code' }}
            aria-current={active === section.key ? 'page' : undefined}
            aria-describedby={section.key === 'requests' && openRequestCount > 0 ? 'repo-open-requests' : undefined}
            className="flex h-full items-center gap-1.5 px-0 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring md:px-3"
            params={params}
            to={section.key === 'requests' && returnToRequest ? returnToRequest.pathname : section.to}
            search={section.key === 'requests' && returnToRequest ? returnToRequest.search : {}}
            hash={section.key === 'requests' && returnToRequest ? returnToRequest.hash : ''}
          >
            {section.label}
            {section.key === 'requests' && openRequestCount > 0 && (
              <span aria-hidden="true" className="text-[11px] tabular-nums text-muted-foreground">
                {openRequestCount}
              </span>
            )}
          </Link>
          {section.key === 'requests' && openRequestCount > 0 && (
            <span hidden id="repo-open-requests">{openRequestCount} open requests</span>
          )}
        </>
      ),
    }),
  )

  return (
    <AppShell
      header={() => (
        <ApplicationTopbar
          facts={[
            ...(!repo || repo.lifecycle_state === 'Ready'
              ? []
              : [{
                  id: 'lifecycle',
                  label: 'Awaiting first push',
                  semantic: 'warning' as const,
                }]),
          ]}
          items={items}
          repository={params}
        >
          <UserButton />
        </ApplicationTopbar>
      )}
    >
      {children}
    </AppShell>
  )
}
