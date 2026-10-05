import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/$owner/$repo/runs')({
  loader: async ({ parentMatchPromise }) => (await parentMatchPromise).loaderData,
})
