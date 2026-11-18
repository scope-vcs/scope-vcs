import { definePlugin } from 'nitro'

import distribution from '../../../cli/distribution/targets.json'
import { cliInstallerResponse } from './cli-installers'

export default definePlugin((nitroApp) => {
  const fetch = nitroApp.fetch
  nitroApp.fetch = (request) =>
    cliInstallerResponse(request, distribution.targets) ?? fetch(request)
})
