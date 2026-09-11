import type {
  AccountSessionResponse,
  CommitFileResponse,
  HistoryEntryFileDiffRequest,
  HistoryEntryRequest,
  HistoryPageRequest,
  OwnerProfileResponse,
  ProjectionPreviewAudience,
  RepoFileResponse,
  RepoSummaryResponse,
  RepositoryMemberPermissions,
  ReviewFileDiffResponse,
  UpdateRepoMetadataRequest,
  Visibility,
} from './types.generated'

export type VisibilityState = Visibility | 'Mixed'

export type CommitSummary = {
  projected_id: string
  logical_commit_id: string
  parent_projected_id: string | null
  author: string | null
  message: string
  change_count: number
}
export type CommitDetail = CommitSummary & {
  audience: ProjectionPreviewAudience
  files_truncated: boolean
  repo_id: string
  view_key: string
  files: CommitFileResponse[]
}

export type ReviewDiffBinarySide = {
  label: 'New' | 'Old'
  oid: string
  sizeBytes: number
}

export type ReviewDiffTextSide = {
  content: string
  label: 'New' | 'Old'
  truncated: boolean
}

export type ReviewDiffOmittedReason = 'hunks' | 'input' | 'lines' | 'output'

export type ReviewDiffPresentation =
  | { kind: 'binary'; sides: ReviewDiffBinarySide[] }
  | { kind: 'empty' }
  | { html: string; kind: 'html' }
  | {
      binary: ReviewDiffBinarySide[]
      kind: 'mixed'
      text: ReviewDiffTextSide[]
    }
  | { kind: 'omitted'; reason: ReviewDiffOmittedReason }

export type ReviewFileDiff = Pick<
  ReviewFileDiffResponse,
  'kind' | 'new_mode' | 'old_mode' | 'path'
> & {
  presentation: ReviewDiffPresentation
}

export type RepoContent = {
  clone_remote_url: string
  files: RepoFileResponse[]
}

export type RepoLiveState = {
  clerk_token_template: string
  event_stream_url: string
  repo: RepoSummaryResponse
}

export type RepoParams = {
  owner: string
  repo: string
}

export type UpdateRepoMetadataInput = RepoParams & UpdateRepoMetadataRequest

export type RunActionInput = RepoParams & {
  run_id: string
}

export type RepoRunHistoryInput = RepoParams & {
  after?: string
  limit?: number
  workflow?: string
}

export type RunStepLogsInput = RunActionInput & {
  after?: number
  before?: number
  attempt_id: string
  step_index: number
}

export type ProfileState = {
  account: AccountSessionResponse
  cliInstallCommands: CliInstallCommands
  profile: OwnerProfileResponse
}

export type CliInstallCommands = {
  posix: string
  windows: string
}

export type CliPlatform = keyof CliInstallCommands

export type CreateRepoInviteInput = RepoParams & {
  email: string
  permissions: RepositoryMemberPermissions
}

export type UpdateRepoMemberInput = RepoParams & {
  member_user_id: string
  permissions: RepositoryMemberPermissions
}

export type DeleteRepoMemberInput = RepoParams & {
  member_user_id: string
}

export type DeleteRepoInviteInput = RepoParams & {
  invite_id: string
}

export type RepoInviteTokenInput = {
  token: string
}

export type HistoryPageInput = RepoParams & HistoryPageRequest
export type HistoryEntryDetailInput = RepoParams & HistoryEntryRequest & {
  entry: string
}
export type HistoryEntryFileDiffInput = RepoParams & HistoryEntryFileDiffRequest & {
  entry: string
}

export type RequestParams = RepoParams & {
  request_id: string
}
