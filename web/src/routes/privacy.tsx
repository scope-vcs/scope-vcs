import privacyPolicy from '@legal/privacy-policy.md?raw'
import { LegalDocumentPage } from '@/features/legal/legal-document-page'
import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/privacy')({
  head: () => ({ meta: [{ title: 'Privacy policy · Scope' }] }),
  component: () => <LegalDocumentPage source={privacyPolicy} />,
})
