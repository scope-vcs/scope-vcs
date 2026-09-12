import type { RepoSummaryResponse } from '@/api/types.generated'

export type RepoSettingsPageState = {
  deleteError: string | null
  deleteTarget: RepoSummaryResponse | null
}

export type RepoSettingsPageAction =
  | { type: 'deleteFailed'; message: string }
  | { type: 'deleteStarted'; repo: RepoSummaryResponse }
  | { type: 'deleteTargetChanged'; repo: RepoSummaryResponse | null }

export const initialRepoSettingsPageState: RepoSettingsPageState = {
  deleteError: null,
  deleteTarget: null,
}

export function repoSettingsPageReducer(
  state: RepoSettingsPageState,
  action: RepoSettingsPageAction,
): RepoSettingsPageState {
  switch (action.type) {
    case 'deleteFailed':
      return { ...state, deleteError: action.message }
    case 'deleteStarted':
      return { ...state, deleteError: null, deleteTarget: action.repo }
    case 'deleteTargetChanged':
      return { ...state, deleteTarget: action.repo }
  }
}
