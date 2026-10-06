import type { RepositoryMemberPermissions, ViewId } from '../../api/types.generated'
import type { RepoViews } from '../../api/repo-views'

export const defaultPermissions: RepositoryMemberPermissions = {
  can_change_file_visibility: false,
  can_push: false,
  view: 'private',
}

export const permissionLabels = [
  {
    description: 'Allows changes to file visibility rules in repository configuration.',
    key: 'can_change_file_visibility',
    label: 'Change file visibility',
  },
  {
    description: 'Allows Git pushes to this repository.',
    key: 'can_push',
    label: 'Push changes',
  },
] as const

export function actionsNeedFullView(view: ViewId, views: Pick<RepoViews, 'full'>) {
  return view !== views.full
}

export function permissionsWithView(
  permissions: RepositoryMemberPermissions,
  view: ViewId,
  views: Pick<RepoViews, 'full'>,
): RepositoryMemberPermissions {
  if (!actionsNeedFullView(view, views)) return { ...permissions, view }
  return { can_change_file_visibility: false, can_push: false, view }
}

export function permissionSummaryText(
  permissions: RepositoryMemberPermissions,
  views: Pick<RepoViews, 'name'>,
) {
  const enabled = permissionLabels.flatMap(({ key, label }) =>
    permissions[key] ? [label.toLowerCase()] : [])
  return [`${views.name(permissions.view)} view`, enabled.length === 0 ? 'No extra actions' : `Also allowed: ${enabled.join(', ')}`].join(' · ')
}
