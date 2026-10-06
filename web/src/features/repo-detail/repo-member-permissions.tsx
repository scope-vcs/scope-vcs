import type { RepositoryMemberPermissions } from '@/api/types.generated'
import type { RepoViews } from '@/api/repo-views'
import { Badge } from '@/components/ui/badge'
import { Switch } from '@/components/ui/switch'
import { Eye } from 'lucide-react'
import { actionsNeedFullView, permissionLabels } from './repo-member-permission-model'
import { MemberViewField } from './repo-member-view-field'

export function MemberAccessSummary({
  permissions,
  viewName,
}: {
  permissions: RepositoryMemberPermissions
  viewName: string
}) {
  return (
    <div className="space-y-3 text-sm">
      <MemberViewRead viewName={viewName} />
      <PermissionSummary permissions={permissions} />
    </div>
  )
}

export function PermissionEditor({
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
  const narrowView = actionsNeedFullView(permissions.view, views)
  return (
    <div className="space-y-2">
      <MemberViewField disabled={disabled} onChange={onChange} permissions={permissions} views={views} />
      {permissionLabels.map((permission) => (
        <label className="flex items-start justify-between gap-4 text-sm" key={permission.key}>
          <span className="min-w-0">
            <span className="block font-medium leading-5">{permission.label}</span>
            <span className="block leading-5 text-muted-foreground">{permission.description}</span>
          </span>
          <Switch
            checked={permissions[permission.key]}
            disabled={disabled || narrowView}
            onCheckedChange={(checked) => onChange({ ...permissions, [permission.key]: checked })}
            type="button"
          />
        </label>
      ))}
    </div>
  )
}

function PermissionSummary({ permissions }: { permissions: RepositoryMemberPermissions }) {
  return (
    <div className="space-y-2">
      {permissionLabels.map((permission) => (
        <div className="flex items-center justify-between gap-3" key={permission.key}>
          <span>{permission.label}</span>
          <Badge variant={permissions[permission.key] ? 'success' : 'neutral'}>
            {permissions[permission.key] ? 'On' : 'Off'}
          </Badge>
        </div>
      ))}
    </div>
  )
}

export function MemberViewRead({ viewName }: { viewName: string }) {
  return (
    <div className="flex items-center justify-between gap-3 text-sm">
      <span className="inline-flex items-center gap-2">
        <Eye className="size-3.5 text-muted-foreground" />
        <span>Read view</span>
      </span>
      <Badge variant="success">{viewName}</Badge>
    </div>
  )
}
