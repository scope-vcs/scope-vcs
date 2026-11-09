import type { ReactNode } from 'react'

declare global {
  interface Window {
    navigations: unknown[]
  }
}

window.navigations = []

export function useNavigate() {
  return (options: unknown) => {
    window.navigations.push(options)
    return Promise.resolve()
  }
}

export function useRouter() {
  return { invalidate: () => Promise.resolve() }
}

export function Link({ children }: { children: ReactNode }) {
  return <a>{children}</a>
}
