import type { RepositoryMemberPermissions } from '../../api/types.generated'

export const defaultPermissions: RepositoryMemberPermissions = {
  can_change_file_visibility: false,
  can_push: false,
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

export function permissionSummaryText(permissions: RepositoryMemberPermissions) {
  const enabled = permissionLabels.flatMap(({ key, label }) =>
    permissions[key] ? [label.toLowerCase()] : [])
  return enabled.length === 0 ? 'No extra actions' : `Also allowed: ${enabled.join(', ')}`
}
