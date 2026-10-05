export function createFileRoute() {
  return (options: object) => ({
    ...options,
    useLoaderData: () => window.accountRouteLoaded,
  })
}

export function redirect() {}
