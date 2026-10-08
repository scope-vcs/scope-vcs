import { createApiClient } from '@/api/client'
import { noContent } from '@/api/http'
import type {
  CompleteBrowserCliLoginInput,
  CompleteCliLoginInput,
  RevokeCliSessionInput,
} from './cli-login-input'
import { ApiRouteTemplates, buildApiPath } from './types.generated'
import {
  BrowserLoginCompleteResponseValidator,
  CliExchangeGrantResponseValidator,
  CliSessionsResponseValidator,
  DeviceLoginCompleteResponseValidator,
} from './validators.generated'

export async function completeCliLoginForRequest(
  data: CompleteCliLoginInput,
) {
  return createApiClient().post(
    buildApiPath(ApiRouteTemplates.cliDeviceLoginComplete, {
      user_code: data.code,
    }),
    DeviceLoginCompleteResponseValidator,
    { auth: 'required' },
  )
}

export async function completeBrowserCliLoginForRequest(
  data: CompleteBrowserCliLoginInput,
) {
  return createApiClient().post(
    buildApiPath(ApiRouteTemplates.cliBrowserLoginComplete, {
      request_id: data.requestId,
    }),
    BrowserLoginCompleteResponseValidator,
    { auth: 'required' },
  )
}

export async function createCliExchangeGrantForRequest() {
  return createApiClient().post(
    ApiRouteTemplates.cliExchangeGrants,
    CliExchangeGrantResponseValidator,
    { auth: 'required' },
  )
}

export async function listCliSessionsForRequest() {
  return createApiClient().get(
    ApiRouteTemplates.cliSessions,
    CliSessionsResponseValidator,
    { auth: 'required' },
  )
}

export async function revokeCliSessionForRequest(data: RevokeCliSessionInput) {
  return createApiClient().delete(
    buildApiPath(ApiRouteTemplates.cliSessionById, {
      session_id: data.sessionId,
    }),
    noContent,
    { auth: 'required' },
  )
}
