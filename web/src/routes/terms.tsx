import termsOfService from '@legal/terms-of-service.md?raw'
import { LegalDocumentPage } from '@/features/legal/legal-document-page'
import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/terms')({
  head: () => ({ meta: [{ title: 'Terms of service · Scope' }] }),
  component: () => <LegalDocumentPage source={termsOfService} />,
})
