import type { RepositoryMemberPermissions } from '@/api/types.generated'
import { Badge } from '@/components/ui/badge'
import { Switch } from '@/components/ui/switch'
import { Eye } from 'lucide-react'
import { builtinViews, viewName } from '@/api/repo-views'
import { permissionLabels } from './repo-member-permission-model'

export function MemberAccessSummary({
  permissions,
}: {
  permissions: RepositoryMemberPermissions
}) {
  return (
    <div className="space-y-3 text-sm">
      <MemberViewRead view={permissions.view} />
      <PermissionSummary permissions={permissions} />
    </div>
  )
}

export function PermissionEditor({
  disabled,
  onChange,
  permissions,
}: {
  disabled?: boolean
  onChange: (permissions: RepositoryMemberPermissions) => void
  permissions: RepositoryMemberPermissions
}) {
  return (
    <div className="space-y-2">
      <label className="flex items-center justify-between gap-4 text-sm">
        <span className="font-medium">View</span>
        <select
          className="rounded border border-border bg-background px-2 py-1 text-foreground"
          disabled={disabled}
          onChange={(event) => onChange({ ...permissions, view: event.target.value })}
          value={permissions.view}
        >
          {builtinViews.map((view) => <option key={view.id} value={view.id}>{view.name}</option>)}
        </select>
      </label>
      {permissionLabels.map((permission) => (
        <label className="flex items-start justify-between gap-4 text-sm" key={permission.key}>
          <span className="min-w-0">
            <span className="block font-medium leading-5">{permission.label}</span>
            <span className="block leading-5 text-muted-foreground">{permission.description}</span>
          </span>
          <Switch
            checked={permissions[permission.key]}
            disabled={disabled}
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

export function MemberViewRead({ view }: { view: string }) {
  return (
    <div className="flex items-center justify-between gap-3 text-sm">
      <span className="inline-flex items-center gap-2">
        <Eye className="size-3.5 text-muted-foreground" />
        <span>Read view</span>
      </span>
      <Badge variant="success">{viewName(view)}</Badge>
    </div>
  )
}
