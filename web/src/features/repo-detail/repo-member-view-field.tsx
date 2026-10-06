import type { RepositoryMemberPermissions } from '@/api/types.generated'
import type { RepoViews } from '@/api/repo-views'
import { ViewSelect } from '../../components/view-select'
import { useId } from 'react'
import { actionsNeedFullView, permissionsWithView } from './repo-member-permission-model'

export function MemberViewField({
  disabled,
  onChange,
  permissions,
  views,
}: {
  disabled?: boolean
  onChange: (permissions: RepositoryMemberPermissions) => void
  permissions: RepositoryMemberPermissions
  views: Pick<RepoViews, 'definitions' | 'full' | 'name'>
}) {
  const id = useId()
  return (
    <div className="space-y-1">
      <div className="flex items-center justify-between gap-4 text-sm">
        <label className="font-medium" htmlFor={id}>View</label>
        <ViewSelect
          className="max-w-48"
          disabled={disabled}
          id={id}
          onChange={(view) => onChange(permissionsWithView(permissions, view, views))}
          value={permissions.view}
          views={views.definitions}
        />
      </div>
      {actionsNeedFullView(permissions.view, views) && views.full && (
        <p className="text-xs leading-5 text-muted-foreground">
          Pushing and changing file visibility need the {views.name(views.full)} view.
        </p>
      )}
    </div>
  )
}
