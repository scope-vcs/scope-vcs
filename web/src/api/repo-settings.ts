import { createApiClient } from '@/api/client'
import type {
  CreateRepoInviteInput,
  RepoInviteInput,
  DeleteRepoMemberInput,
  RepoParams,
  RepoInviteTokenInput,
  UpdateRepoMemberInput,
  UpdateRepoMetadataInput,
} from './types'
import type {
  AcceptRepositoryInviteResponse,
  RepositoryCollaborationResponse,
  RepositoryInviteResponse,
  RepositoryInviteLandingResponse,
  RepositoryInviteLinkResponse,
  RepositoryMemberResponse,
  RepoSummaryResponse,
} from './types.generated'
import { repoRoute } from './paths'
import { ApiRouteTemplates, buildApiPath } from './types.generated'
import { apiValidators } from './validators.generated'

export async function updateRepoMetadataForRequest(
  data: UpdateRepoMetadataInput,
): Promise<RepoSummaryResponse> {
  return createApiClient().patch(
    repoRoute(ApiRouteTemplates.repoMetadata, data),
    apiValidators.RepoSummaryResponse,
    {
      auth: 'required',
      body: { description: data.description, website_url: data.website_url },
    },
  )
}

export async function deleteRepoForRequest(data: RepoParams) {
  return createApiClient().delete(
    repoRoute(ApiRouteTemplates.repo, data),
    apiValidators.DeleteRepoResponse,
    { auth: 'required' },
  )
}

export async function loadRepoCollaborationForRequest(
  data: RepoParams,
  signal?: AbortSignal,
): Promise<RepositoryCollaborationResponse> {
  return createApiClient().get(
    repoRoute(ApiRouteTemplates.repoMembers, data),
    apiValidators.RepositoryCollaborationResponse,
    { auth: 'required', signal },
  )
}

export async function createRepoInviteForRequest(
  data: CreateRepoInviteInput,
): Promise<RepositoryInviteResponse> {
  return createApiClient().post(
    repoRoute(ApiRouteTemplates.repoInvites, data),
    apiValidators.RepositoryInviteResponse,
    {
      auth: 'required',
      body: {
        email: data.email,
        permissions: data.permissions,
      },
    },
  )
}

export async function updateRepoMemberForRequest(
  data: UpdateRepoMemberInput,
): Promise<RepositoryMemberResponse> {
  return createApiClient().patch(
    buildApiPath(ApiRouteTemplates.repoMember, {
      owner: data.owner,
      repo: data.repo,
      member_user_id: data.member_user_id,
    }),
    apiValidators.RepositoryMemberResponse,
    {
      auth: 'required',
      body: {
        permissions: data.permissions,
      },
    },
  )
}

export async function deleteRepoMemberForRequest(
  data: DeleteRepoMemberInput,
): Promise<RepositoryMemberResponse> {
  return createApiClient().delete(
    buildApiPath(ApiRouteTemplates.repoMember, {
      owner: data.owner,
      repo: data.repo,
      member_user_id: data.member_user_id,
    }),
    apiValidators.RepositoryMemberResponse,
    { auth: 'required' },
  )
}

export async function deleteRepoInviteForRequest(
  data: RepoInviteInput,
): Promise<RepositoryInviteResponse> {
  return createApiClient().delete(
    buildApiPath(ApiRouteTemplates.repoInvite, {
      owner: data.owner,
      repo: data.repo,
      invite_id: data.invite_id,
    }),
    apiValidators.RepositoryInviteResponse,
    { auth: 'required' },
  )
}

export async function createRepoInviteLinkForRequest(
  data: RepoInviteInput,
): Promise<RepositoryInviteLinkResponse> {
  return createApiClient().post(
    buildApiPath(ApiRouteTemplates.repoInviteLinks, {
      owner: data.owner,
      repo: data.repo,
      invite_id: data.invite_id,
    }),
    apiValidators.RepositoryInviteLinkResponse,
    { auth: 'required' },
  )
}

export async function sendRepoInviteEmailForRequest(
  data: RepoInviteInput,
): Promise<RepositoryInviteResponse> {
  return createApiClient().post(
    buildApiPath(ApiRouteTemplates.repoInviteEmails, {
      owner: data.owner,
      repo: data.repo,
      invite_id: data.invite_id,
    }),
    apiValidators.RepositoryInviteResponse,
    { auth: 'required' },
  )
}

export async function loadRepoInviteForRequest(
  data: RepoInviteTokenInput,
): Promise<RepositoryInviteLandingResponse> {
  return createApiClient().get(
    buildApiPath(ApiRouteTemplates.repositoryInvite, { token: data.token }),
    apiValidators.RepositoryInviteLandingResponse,
    { auth: 'optional' },
  )
}

export async function acceptRepoInviteForRequest(
  data: RepoInviteTokenInput,
): Promise<AcceptRepositoryInviteResponse> {
  return createApiClient().post(
    buildApiPath(ApiRouteTemplates.repositoryInviteAccept, {
      token: data.token,
    }),
    apiValidators.AcceptRepositoryInviteResponse,
    { auth: 'required' },
  )
}
