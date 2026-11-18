import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { afterEach, test } from 'node:test'

import type { CliRelease } from './cli-installer-scripts'
import { cliInstallerResponse, configuredCliRelease } from './cli-installers'

const originalApiUrl = process.env.SCOPE_API_PUBLIC_URL
const targets = [
  { artifact: 'scope-x86_64-unknown-linux-gnu.tar.gz', os: 'linux', arch: 'x64' },
  { artifact: 'scope-aarch64-apple-darwin.tar.gz', os: 'macos', arch: 'arm64' },
  { artifact: 'scope-x86_64-pc-windows-msvc.tar.gz', os: 'windows', arch: 'x64' },
]
const release: CliRelease = {
  tag: 'cli-0123456789ab',
  artifacts: {
    'scope-x86_64-unknown-linux-gnu.tar.gz': 'a'.repeat(64),
    'scope-aarch64-apple-darwin.tar.gz': 'b'.repeat(64),
    'scope-x86_64-pc-windows-msvc.tar.gz': 'c'.repeat(64),
  },
}

afterEach(() => {
  if (originalApiUrl === undefined) delete process.env.SCOPE_API_PUBLIC_URL
  else process.env.SCOPE_API_PUBLIC_URL = originalApiUrl
})

function install(path: string, loaded: CliRelease | null = release) {
  return cliInstallerResponse(
    new Request(`https://cli.scope.example${path}`),
    targets,
    () => loaded,
  )
}

test('installers pin each binary to the release checksum and download through the tracked API route', async () => {
  process.env.SCOPE_API_PUBLIC_URL = 'https://api.scope.example/'

  const shell = install('/install.sh')
  const powershell = install('/install.ps1')

  assert.equal(shell?.status, 200)
  assert.equal(shell.headers.get('content-type'), 'text/x-shellscript; charset=utf-8')
  const sh = await shell.text()
  execFileSync('sh', ['-n'], { input: sh })
  assert.match(sh, /^download_url='https:\/\/api\.scope\.example\/cli\/downloads\/cli-0123456789ab'$/m)
  assert.match(sh, /Linux-x86_64\|Linux-amd64\)\n {4}artifact="scope-x86_64-unknown-linux-gnu\.tar\.gz"\n {4}expected="a{64}"/)
  assert.match(sh, /Darwin-arm64\|Darwin-aarch64\)\n {4}artifact="scope-aarch64-apple-darwin\.tar\.gz"\n {4}expected="b{64}"/)
  assert.match(sh, /curl -fsSL "\$download_url\/\$\{artifact\}\?installer=sh"/)
  assert.doesNotMatch(sh, /\.sha256|windows/)

  assert.equal(powershell?.status, 200)
  const ps1 = await powershell.text()
  assert.match(ps1, /\$artifact = "scope-x86_64-pc-windows-msvc\.tar\.gz"\n {4}\$expected = "c{64}"/)
  assert.match(ps1, /Invoke-WebRequest -Uri "\$downloadUrl\/\$\{artifact\}\?installer=ps1"/)
  assert.doesNotMatch(ps1, /\.sha256|apple-darwin/)
})

test('installers are unavailable without a release and refuse a release that lacks a checksum', async () => {
  assert.equal(install('/install.sh', configuredCliRelease(''))?.status, 503)
  assert.equal(install('/'), null)
  assert.deepEqual(configuredCliRelease(JSON.stringify(release)), release)
  assert.throws(
    () => install('/install.sh', { tag: release.tag, artifacts: {} }),
    /no checksum for scope-x86_64-unknown-linux-gnu/,
  )
  assert.throws(
    () => install('/install.ps1', { ...release, tag: 'cli-$(injected)' }),
    /not a published CLI release/,
  )
  assert.throws(() => configuredCliRelease('{"tag":"cli-1"}'), /does not describe/)
})

test('a malformed release setting fails only the installers', () => {
  const malformed = () => configuredCliRelease('{"tag":')
  const request = (path: string) =>
    cliInstallerResponse(
      new Request(`https://scope.example${path}`),
      targets,
      malformed,
    )

  assert.equal(request('/owner/repo'), null)
  assert.equal(request('/install.sh')?.status, 503)
  assert.equal(request('/install.ps1')?.status, 503)
})
