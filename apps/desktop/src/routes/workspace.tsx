import { createFileRoute, redirect } from "@tanstack/react-router"

import { session } from "@/features/session"
import { RouteError, RouteNotFound } from "@/features/workspace/route-failures"

export const Route = createFileRoute("/workspace")({
  // No workspace without an open connection: there is nothing to show, and a
  // screen that pretends otherwise would invite queries against nothing.
  beforeLoad: () => {
    if (!session.state.open) throw redirect({ to: "/" })
  },
  // The workspace itself is `WorkspaceHost`, mounted by the root so that it
  // outlives a visit to the start screen; this route only shows it.
  component: () => null,
  errorComponent: RouteError,
  notFoundComponent: RouteNotFound,
})
