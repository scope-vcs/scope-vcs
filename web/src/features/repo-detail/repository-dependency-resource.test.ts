import assert from 'node:assert/strict'
import test from 'node:test'
import type {
  RepositoryActor,
  RepositoryDependencyCheckResponse,
  RepoSummaryResponse,
} from '../../api/types.generated'
import {
  createRepositoryDependencyResource,
  repositoryDependencyIdentity,
  repositoryDependencyResource,
} from './repository-dependency-resource'

const unsupported: RepositoryDependencyCheckResponse = {
  error: null,
  report: null,
  status: 'Unsupported',
}

function repo(actor: RepositoryActor): RepoSummaryResponse {
  return {
    access: {
      actor,
      can_apply_changes: actor !== 'Public',
      can_change_file_visibility: actor !== 'Public',
      can_delete_repo: actor === 'Owner',
      can_manage_members: actor === 'Owner',
      can_push: actor !== 'Public',
      can_read_private_files: actor !== 'Public',
    },
    change_version: 4,
    description: null,
    git_remote_url: 'https://example.com/acme/repo.git',
    id: 'repo-1',
    lifecycle_state: 'Ready',
    name: 'repo',
    open_request_count: 0,
    owner_handle: 'acme',
    request_permissions: {
      can_start_request: actor !== 'Public',
    },
    website_url: null,
  }
}

test('reuses a retained report until its repository event invalidates it', async () => {
  repositoryDependencyResource.clear()
  const identity = repositoryDependencyIdentity(repo('Member'), 'viewer-1')!
  let loads = 0
  const load = async () => {
    loads += 1
    return unsupported
  }

  await repositoryDependencyResource.ensure(identity, '4', load)
  await repositoryDependencyResource.ensure(identity, '4', load)
  assert.equal(loads, 1)

  repositoryDependencyResource.invalidate(identity)
  assert.equal(repositoryDependencyResource.peek(identity), unsupported)
  await repositoryDependencyResource.ensure(identity, '4', load)
  assert.equal(loads, 2)
})

test('polls pending and updating checks until a terminal result is retained', async () => {
  let scheduled: (() => void) | null = null
  let canceled = 0
  const resource = createRepositoryDependencyResource((poll) => {
    scheduled = () => {
      scheduled = null
      poll()
    }
    return () => {
      canceled += 1
      scheduled = null
    }
  })
  const identity = 'repo-viewer-access'
  let status: RepositoryDependencyCheckResponse['status'] = 'Pending'
  let loads = 0
  const load = async (): Promise<RepositoryDependencyCheckResponse> => {
    loads += 1
    return { error: null, report: null, status }
  }

  await resource.ensure(identity, '4', load)
  assert.equal(loads, 1)
  assert.notEqual(scheduled, null)

  const poll = scheduled as unknown as () => void
  poll()
  assert.equal(resource.getSnapshot(identity).stale, true)
  assert.equal(resource.peek(identity)?.status, 'Pending')

  status = 'Ready'
  await resource.ensure(identity, '4', load)
  assert.equal(loads, 2)
  assert.equal(scheduled, null)
  assert.equal(resource.peek(identity)?.status, 'Ready')
  assert.equal(canceled, 0)
})

test('an event cancels a pending poll while retaining the previous result', async () => {
  let canceled = 0
  const resource = createRepositoryDependencyResource(() => () => {
    canceled += 1
  })
  const pending: RepositoryDependencyCheckResponse = {
    error: null,
    report: null,
    status: 'Updating',
  }

  await resource.ensure('repo', '4', async () => pending)
  resource.invalidate('repo')

  assert.equal(canceled, 1)
  assert.equal(resource.getSnapshot('repo').stale, true)
  assert.equal(resource.peek('repo'), pending)
})

test('retries a failed poll without dropping its retained result', async () => {
  let scheduled: (() => void) | null = null
  const resource = createRepositoryDependencyResource((poll) => {
    scheduled = () => {
      scheduled = null
      poll()
    }
    return () => {
      scheduled = null
    }
  })
  const updating: RepositoryDependencyCheckResponse = {
    error: null,
    report: null,
    status: 'Updating',
  }
  await resource.ensure('repo', '4', async () => updating)
  const firstPoll = scheduled as unknown as () => void
  firstPoll()

  await resource.ensure('repo', '4', async () => {
    throw new Error('temporary outage')
  })

  assert.equal(resource.peek('repo'), updating)
  assert.equal(resource.getSnapshot('repo').error instanceof Error, true)
  assert.notEqual(scheduled, null)
})

test('polls durable job failures less often than active checks', async () => {
  const delays: number[] = []
  let scheduled: (() => void) | null = null
  const resource = createRepositoryDependencyResource((poll, delayMs) => {
    delays.push(delayMs)
    scheduled = poll
    return () => {
      scheduled = null
    }
  })
  await resource.ensure('repo', '4', async () => ({
    error: null,
    report: null,
    status: 'Pending',
  }))
  const pendingPoll = scheduled as unknown as () => void
  pendingPoll()
  await resource.ensure('repo', '4', async () => ({
    error: 'analyzer failed',
    report: null,
    status: 'Failed',
  }))

  assert.equal(delays.length, 2)
  assert.equal(delays[1] > delays[0], true)
})

test('hides public access and isolates changes in viewer or maintainer access', () => {
  const owner = repositoryDependencyIdentity(repo('Owner'), 'viewer-1')
  const member = repositoryDependencyIdentity(repo('Member'), 'viewer-1')
  const otherViewer = repositoryDependencyIdentity(repo('Member'), 'viewer-2')

  assert.equal(repositoryDependencyIdentity(repo('Public'), null), null)
  assert.equal(repositoryDependencyIdentity(repo('Owner'), null), null)
  assert.notEqual(owner, member)
  assert.notEqual(member, otherViewer)
})
