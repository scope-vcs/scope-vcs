import { createApiClient } from '@/api/client'
import { HttpError, noContent } from '@/api/http'
import { ApiRouteTemplates } from './types.generated'

export type DeleteAccountResult =
  | { status: 'deleted' }
  /** Repository ids, as `owner/name`, that other members still use. */
  | { status: 'blocked'; repositories: string[] }

export async function deleteAccountForRequest(): Promise<DeleteAccountResult> {
  try {
    await createApiClient().delete(ApiRouteTemplates.account, noContent, { auth: 'required' })
    return { status: 'deleted' }
  } catch (error) {
    if (error instanceof HttpError && error.response.code === 'shared_repositories') {
      return { repositories: error.response.fields?.repositories ?? [], status: 'blocked' }
    }
    throw error
  }
}
