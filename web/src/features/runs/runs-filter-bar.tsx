import type { RepoParams } from '@/api/types'
import type { RepositoryRunWorkflowListResponse } from '@/api/types.generated'
import { Select } from '@/components/ui/select'
import { useNavigate } from '@tanstack/react-router'
import {
  RUN_STATUS_FILTER_OPTIONS,
  type RunStatusFilter,
} from './runs-filter-model'

export function RunsFilterBar({
  onStatusFilterChange,
  params,
  selectedWorkflow,
  showWorkflowFilter,
  statusFilter,
  workflows,
}: {
  onStatusFilterChange: (filter: RunStatusFilter) => void
  params: RepoParams
  selectedWorkflow?: string
  showWorkflowFilter: boolean
  statusFilter: RunStatusFilter
  workflows: RepositoryRunWorkflowListResponse['workflows']
}) {
  const navigate = useNavigate()

  return (
    <div className="flex flex-wrap items-center gap-2">
      {showWorkflowFilter ? (
        <Select
          aria-label="Filter by workflow"
          className="max-w-44"
          onChange={(event) => {
            const value = event.target.value
            if (value === '') {
              void navigate({ params, to: '/$owner/$repo/runs' })
              return
            }
            void navigate({
              params: { ...params, workflow: value },
              to: '/$owner/$repo/runs/workflows/$workflow',
            })
          }}
          value={selectedWorkflow ?? ''}
        >
          <option value="">All workflows</option>
          {workflows.map((item) => (
            <option key={item.key} value={item.key}>
              {item.name}
            </option>
          ))}
        </Select>
      ) : null}
      <Select
        aria-label="Filter by status"
        onChange={(event) => onStatusFilterChange(event.target.value as RunStatusFilter)}
        value={statusFilter}
      >
        {RUN_STATUS_FILTER_OPTIONS.map((option) => (
          <option key={option.value} value={option.value}>
            {option.label}
          </option>
        ))}
      </Select>
    </div>
  )
}
