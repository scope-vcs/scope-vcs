import { serverLog } from './log'
import {
  posixInstallScript,
  windowsInstallScript,
  type CliDistributionTarget,
  type CliRelease,
} from './cli-installer-scripts'

const releaseEnv = 'SCOPE_CLI_RELEASE'
const publicApiUrlEnv = 'SCOPE_API_PUBLIC_URL'
const localApiOrigin = 'http://localhost:8080'

const installers = {
  '/install.sh': {
    contentType: 'text/x-shellscript; charset=utf-8',
    script: posixInstallScript,
  },
  '/install.ps1': {
    contentType: 'text/plain; charset=utf-8',
    script: windowsInstallScript,
  },
} as const

export function configuredCliRelease(
  configured = process.env[releaseEnv],
): CliRelease | null {
  if (!configured) return null
  const release: unknown = JSON.parse(configured)
  const { tag, artifacts } = (release ?? {}) as {
    tag?: unknown
    artifacts?: unknown
  }
  if (
    typeof tag !== 'string' ||
    typeof artifacts !== 'object' ||
    artifacts === null
  ) {
    throw new Error(`${releaseEnv} does not describe a CLI release`)
  }
  return { tag, artifacts: artifacts as Record<string, string> }
}

export function cliInstallerResponse(
  request: Request,
  targets: readonly CliDistributionTarget[],
  readRelease: () => CliRelease | null = configuredCliRelease,
): Response | null {
  const pathname = new URL(request.url).pathname
  if (pathname !== '/install.sh' && pathname !== '/install.ps1') return null
  if (request.method !== 'GET' && request.method !== 'HEAD') {
    return new Response(null, { status: 405, headers: { allow: 'GET, HEAD' } })
  }
  let release: CliRelease | null
  try {
    release = readRelease()
  } catch (error) {
    serverLog('error', 'The CLI release setting is malformed', {
      error: error instanceof Error ? error.message : String(error),
    })
    release = null
  }
  if (!release) {
    return new Response('The Scope CLI installer is not available here.\n', {
      status: 503,
      headers: { 'cache-control': 'no-store' },
    })
  }
  const installer = installers[pathname]
  const apiOrigin = (process.env[publicApiUrlEnv] ?? localApiOrigin).replace(
    /\/$/,
    '',
  )
  const script = installer.script(apiOrigin, release, targets)
  return new Response(request.method === 'HEAD' ? null : script, {
    headers: {
      'cache-control': 'no-store',
      'content-type': installer.contentType,
    },
  })
}
