import * as assert from 'node:assert/strict'
import { test } from 'node:test'
import { arrayOf } from './http'
import { ErrorResponseValidator, RepoChangeEventValidator, RepoFileResponseValidator } from './validators.generated'

test('generated validators follow enum, optional, and unknown-field Serde policy', () => {
  assert.equal(ErrorResponseValidator({
    code: 'internal',
    message: 'failed',
    retryable: false,
  }), true)
  assert.equal(ErrorResponseValidator({
    code: 'internal',
    extra_server_field: 'allowed until Rust denies unknown fields',
    message: 'failed',
    retryable: false,
  }), true)
  assert.equal(ErrorResponseValidator({
    code: 'not-a-real-code',
    message: 'failed',
    retryable: false,
  }), false)
  assert.equal(ErrorResponseValidator({
    code: 'internal',
    message: 'failed',
  }), false)
})

test('generated validators enforce arrays and JavaScript safe integers', () => {
  const validateRepoFiles = arrayOf(RepoFileResponseValidator)
  assert.equal(validateRepoFiles([{
    oid: '0123456789abcdef0123456789abcdef01234567',
    path: '/README.md',
    tracked: true,
    label: 'public',
  }]), true)
  assert.equal(validateRepoFiles([{
    oid: '0123456789abcdef0123456789abcdef01234567',
    path: '/README.md',
    tracked: 'yes',
    label: 'public',
  }]), false)

  const connected = {
    incarnation_id: 'incarnation-1',
    kind: 'Connected',
    repo_id: 'owner/repo',
    version: Number.MAX_SAFE_INTEGER,
  }
  assert.equal(RepoChangeEventValidator(connected), true)
  assert.equal(RepoChangeEventValidator({
    ...connected,
    version: Number.MAX_SAFE_INTEGER + 1,
  }), false)
})

test('generated RepoChangeEvent validator accepts dependency invalidation events', () => {
  assert.equal(RepoChangeEventValidator({
    incarnation_id: 'incarnation-1',
    kind: 'DependenciesChanged',
    repo_id: 'owner/repo',
    version: 2,
  }), true)
})
