import { createRouter as createTanStackRouter } from "@tanstack/react-router"

import { createQueryClient } from "./lib/query-client"
import { routeTree } from "./routeTree.gen"

export function getRouter() {
  const queryClient = createQueryClient()

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
