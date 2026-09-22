import { getCliInstallConnection } from '@/api/client'
import type { CliInstallCommands, CliInstallState } from '@/api/types'
import { detectCliPlatform } from '@/lib/cli-platform'

export function buildCliInstallCommands(): CliInstallCommands {
  const baseUrl = getCliInstallConnection()
  return {
    posix: `curl -fsSL ${baseUrl}/install.sh | sh`,
    windows: `irm ${baseUrl}/install.ps1 | iex`,
  }
}

/** Install commands plus the platform to preselect for the current request. */
export async function loadCliInstallStateForRequest(): Promise<CliInstallState> {
  const { getRequestHeader } = await import('@tanstack/react-start/server')
  const platformHeader = getRequestHeader('sec-ch-ua-platform')
    ?? getRequestHeader('user-agent')

  return {
    cliInstallCommands: buildCliInstallCommands(),
    initialCliPlatform: detectCliPlatform(platformHeader),
  }
}
