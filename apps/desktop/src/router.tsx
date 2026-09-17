import { QueryClient } from "@tanstack/react-query"
import { createRouter as createTanStackRouter } from "@tanstack/react-router"

import { routeTree } from "./routeTree.gen"

export function getRouter() {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: {
        // The backend is local: a failed call is an answer, not a flaky
        // network. Retrying hides it, and an ambiguous write must never be
        // replayed (I-13).
        retry: false,
        refetchOnWindowFocus: false,
      },
      mutations: { retry: false },
    },
  })

  return createTanStackRouter({
    routeTree,
    context: { queryClient },
    // Nothing to scroll back to in a fixed workbench layout.
    scrollRestoration: false,
    defaultPreload: false,
  })
}

declare module "@tanstack/react-router" {
  interface Register {
    router: ReturnType<typeof getRouter>
  }
}
