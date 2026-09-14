import { StartClient } from '@tanstack/react-start/client'
import { hydrateRoot } from 'react-dom/client'
import { reportFrontendError } from './analytics/diagnostics'

hydrateRoot(document, <StartClient />, {
  onRecoverableError: (error) => reportFrontendError(error, 'hydration'),
})
