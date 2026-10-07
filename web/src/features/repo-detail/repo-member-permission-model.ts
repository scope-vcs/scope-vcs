import type { RepositoryMemberPermissions, ViewId } from '../../api/types.generated'
import type { RepoViews } from '../../api/repo-views'

export type MemberPermissionKey = 'can_change_file_visibility' | 'can_push'

export const defaultPermissions: RepositoryMemberPermissions = {
  can_change_file_visibility: false,
  can_push: false,
  view: 'private',
}

export const permissionLabels = [
  { key: 'can_change_file_visibility', label: 'Change file visibility' },
  { key: 'can_push', label: 'Push changes' },
] as const satisfies readonly { key: MemberPermissionKey; label: string }[]

export function isNarrowerView(view: ViewId, views: Pick<RepoViews, 'full'>) {
  return view !== views.full
}

export function permissionAvailable(
  key: MemberPermissionKey,
  view: ViewId,
  views: Pick<RepoViews, 'full'>,
) {
  return key === 'can_push' || !isNarrowerView(view, views)
}

export function permissionDescription(
  key: MemberPermissionKey,
  view: ViewId,
  views: Pick<RepoViews, 'full'>,
) {
  if (key === 'can_change_file_visibility') {
    return 'Allows changes to file visibility rules in repository configuration.'
  }
  return isNarrowerView(view, views)
    ? 'Allows Git pushes. Pushes to main land as an auto-merged request in this view.'
    : 'Allows Git pushes to this repository.'
}

export function permissionsWithView(
  permissions: RepositoryMemberPermissions,
  view: ViewId,
  views: Pick<RepoViews, 'full'>,
): RepositoryMemberPermissions {
  return {
    can_change_file_visibility:
      permissions.can_change_file_visibility &&
      permissionAvailable('can_change_file_visibility', view, views),
    can_push: permissions.can_push,
    view,
  }
}

export function permissionSummaryText(
  permissions: RepositoryMemberPermissions,
  views: Pick<RepoViews, 'full' | 'name'>,
) {
  const enabled = permissionLabels.flatMap(({ key, label }) =>
    permissions[key] ? [summaryLabel(key, label, permissions.view, views)] : [])
  return [
    `${views.name(permissions.view)} view`,
    enabled.length === 0 ? 'No extra actions' : `Also allowed: ${enabled.join(', ')}`,
  ].join(' · ')
}

function summaryLabel(
  key: MemberPermissionKey,
  label: string,
  view: ViewId,
  views: Pick<RepoViews, 'full'>,
) {
  const text = label.toLowerCase()
  return key === 'can_push' && isNarrowerView(view, views) ? `${text} as requests` : text
}
