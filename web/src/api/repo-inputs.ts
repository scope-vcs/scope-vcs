import type {
  CreateRepoInviteInput,
  DeleteRepoInviteInput,
  DeleteRepoMemberInput,
  RepoInviteTokenInput,
  UpdateRepoMemberInput,
  UpdateRepoMetadataInput,
} from './types'
import type { RepositoryMemberPermissions } from './types.generated'
import { parseRepoParams } from './repo-params'

export function parseUpdateRepoMetadataInput(input: unknown): UpdateRepoMetadataInput {
  const params = parseRepoParams(input)
  const data = input as Partial<UpdateRepoMetadataInput>
  if (
    data.description !== null && typeof data.description !== 'string' ||
    data.website_url !== null && typeof data.website_url !== 'string'
  ) {
    throw new Error('Repository description and website must be text or empty.')
  }
  return { ...params, description: data.description, website_url: data.website_url }
}

export function parseCreateRepoInviteInput(input: unknown): CreateRepoInviteInput {
  const params = parseRepoParams(input)
  const data = input as Partial<CreateRepoInviteInput>
  const email = typeof data.email === 'string' ? data.email.trim() : ''
  if (!email) {
    throw new Error('Invite email is required.')
  }
  return { ...params, email, permissions: parseMemberPermissions(data.permissions) }
}

export function parseUpdateRepoMemberInput(input: unknown): UpdateRepoMemberInput {
  const params = parseRepoParams(input)
  const data = input as Partial<UpdateRepoMemberInput>
  return {
    ...params,
    member_user_id: requiredId(data.member_user_id, 'Repository member route is incomplete.'),
    permissions: parseMemberPermissions(data.permissions),
  }
}

export function parseDeleteRepoMemberInput(input: unknown): DeleteRepoMemberInput {
  const params = parseRepoParams(input)
  const data = input as Partial<DeleteRepoMemberInput>
  return {
    ...params,
    member_user_id: requiredId(data.member_user_id, 'Repository member route is incomplete.'),
  }
}

export function parseDeleteRepoInviteInput(input: unknown): DeleteRepoInviteInput {
  const params = parseRepoParams(input)
  const data = input as Partial<DeleteRepoInviteInput>
  return {
    ...params,
    invite_id: requiredId(data.invite_id, 'Repository invite route is incomplete.'),
  }
}

export function parseRepoInviteTokenInput(input: unknown): RepoInviteTokenInput {
  const data = input as Partial<RepoInviteTokenInput> | null
  return { token: requiredId(data?.token, 'Invite token is missing.') }
}

function requiredId(value: unknown, message: string) {
  const id = typeof value === 'string' ? value.trim() : ''
  if (!id) {
    throw new Error(message)
  }
  return id
}

function parseMemberPermissions(input: unknown): RepositoryMemberPermissions {
  const data = input as Partial<RepositoryMemberPermissions> | null
  return {
    can_apply_changes: false,
    can_change_file_visibility: false,
    can_push: data?.can_push === true,
  }
}
