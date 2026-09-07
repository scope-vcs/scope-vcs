import type { RepoParams } from '@/api/types'
import {
  ApplicationTopbar,
  type TopbarItem,
} from '@/components/application-topbar'
import { AppShell } from '@/components/app-shell'
import {
  activeRepoSection,
  repoSectionsForActor,
} from '@/components/repo-section-model'
import { useRepoLayout } from '@/features/repo-detail/repo-layout-context'
import { UserButton } from '@clerk/tanstack-react-start'
import { Link, useLocation, useRouter } from '@tanstack/react-router'
import type { ReactNode } from 'react'

export function RepoShell({
  children,
  params,
}: {
  children: ReactNode
  params: RepoParams
}) {
  const { repo } = useRepoLayout()
  const router = useRouter()
  const pathname = useLocation({ select: (location) => location.pathname })
  const sections = repoSectionsForActor(repo.access.actor)
  const active = activeRepoSection((to) => {
    const target = router.buildLocation({ params, to }).pathname
    return pathname === target || pathname.startsWith(`${target}/`)
  })
  const items = sections.map<TopbarItem>(
    (section) => ({
      active: active === section.key,
      label: section.label,
      node: (
        <>
          <Link
            activeOptions={{ exact: section.key === 'code' }}
            aria-current={active === section.key ? 'page' : undefined}
            aria-describedby={section.key === 'requests' && repo.open_request_count > 0 ? 'repo-open-requests' : undefined}
            className="flex h-full items-center gap-1.5 px-0 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring md:px-3"
            params={params}
            to={section.to}
          >
            {section.label}
            {section.key === 'requests' && repo.open_request_count > 0 && (
              <span aria-hidden="true" className="text-[11px] tabular-nums text-muted-foreground">
                {repo.open_request_count}
              </span>
            )}
          </Link>
          {section.key === 'requests' && repo.open_request_count > 0 && (
            <span hidden id="repo-open-requests">{repo.open_request_count} open requests</span>
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
            ...(repo.lifecycle_state === 'Ready'
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
