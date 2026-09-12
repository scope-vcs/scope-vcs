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
} from './repository-dependency-resource'

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
    website_url: null,
  }
}

function pollScheduler() {
  let scheduled: (() => void) | null = null
  let canceled = 0
  const delays: number[] = []
  return {
    schedule(poll: () => void, delayMs: number) {
      delays.push(delayMs)
      scheduled = poll
      return () => {
        canceled += 1
        scheduled = null
      }
    },
    fire() {
      assert.ok(scheduled, 'expected a scheduled poll')
      const poll = scheduled
      scheduled = null
      poll()
    },
    get pending() { return scheduled !== null },
    get canceled() { return canceled },
    delays,
  }
}

test('polls a pending check until a terminal result is retained', async () => {
  const scheduler = pollScheduler()
  const resource = createRepositoryDependencyResource(scheduler.schedule)
  const identity = 'repo-viewer-access'
  let status: RepositoryDependencyCheckResponse['status'] = 'Pending'
  let loads = 0
  const load = async (): Promise<RepositoryDependencyCheckResponse> => {
    loads += 1
    return { error: null, report: null, status }
  }

  await resource.ensure(identity, '4', load)
  assert.equal(loads, 1)
  assert.equal(scheduler.pending, true)

  scheduler.fire()
  assert.equal(resource.getSnapshot(identity).stale, true)
  assert.equal(resource.peek(identity)?.status, 'Pending')

  status = 'Ready'
  await resource.ensure(identity, '4', load)
  assert.equal(loads, 2)
  assert.equal(scheduler.pending, false)
  assert.equal(resource.peek(identity)?.status, 'Ready')
  assert.equal(scheduler.canceled, 0)
})

test('an event cancels a pending poll while retaining the previous result', async () => {
  const scheduler = pollScheduler()
  const resource = createRepositoryDependencyResource(scheduler.schedule)
  const pending: RepositoryDependencyCheckResponse = {
    error: null,
    report: null,
    status: 'Updating',
  }

  await resource.ensure('repo', '4', async () => pending)
  resource.invalidate('repo')

  assert.equal(scheduler.canceled, 1)
  assert.equal(resource.getSnapshot('repo').stale, true)
  assert.equal(resource.peek('repo'), pending)
})

test('retries a failed poll without dropping its retained result', async () => {
  const scheduler = pollScheduler()
  const resource = createRepositoryDependencyResource(scheduler.schedule)
  const updating: RepositoryDependencyCheckResponse = {
    error: null,
    report: null,
    status: 'Updating',
  }
  await resource.ensure('repo', '4', async () => updating)
  scheduler.fire()

  await resource.ensure('repo', '4', async () => {
    throw new Error('temporary outage')
  })

  assert.equal(resource.peek('repo'), updating)
  assert.equal(resource.getSnapshot('repo').error instanceof Error, true)
  assert.equal(scheduler.pending, true)
})

test('polls durable job failures less often than active checks', async () => {
  const scheduler = pollScheduler()
  const resource = createRepositoryDependencyResource(scheduler.schedule)
  await resource.ensure('repo', '4', async () => ({
    error: null,
    report: null,
    status: 'Pending',
  }))
  scheduler.fire()
  await resource.ensure('repo', '4', async () => ({
    error: 'analyzer failed',
    report: null,
    status: 'Failed',
  }))

  assert.equal(scheduler.delays.length, 2)
  assert.equal(scheduler.delays[1] > scheduler.delays[0], true)
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
