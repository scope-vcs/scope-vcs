import { createApiClient } from '@/api/client'
import { loadCliInstallStateForRequest } from '@/api/cli-install'
import type { ProfileState } from './types'
import { ApiRouteTemplates, buildApiPath } from './types.generated'
import {
  AccountSessionResponseValidator,
  OwnerProfileResponseValidator,
} from './validators.generated'

export async function loadOwnerProfileForRequest(
  handle: string,
): Promise<ProfileState> {
  const api = createApiClient()
  const [account, profile, cliInstall] = await Promise.all([
    loadAccountSessionForRequest(),
    api.get(
      buildApiPath(ApiRouteTemplates.ownerRepositories, { handle }),
      OwnerProfileResponseValidator,
      { auth: 'optional' },
    ),
    loadCliInstallStateForRequest(),
  ])

  return { ...cliInstall, account, profile }
}

export async function loadAccountSessionForRequest() {
  const api = createApiClient()
  return api.get(
    buildApiPath(ApiRouteTemplates.accountSession),
    AccountSessionResponseValidator,
    { auth: 'optional' },
  )
}

export async function loadAuthenticatedAccountForRequest() {
  const api = createApiClient()
  return api.get(
    buildApiPath(ApiRouteTemplates.accountSession),
    AccountSessionResponseValidator,
    { auth: 'required' },
  )
}
