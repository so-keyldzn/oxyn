import { createFileRoute } from "@tanstack/react-router"

import { RecoveryScreen } from "@/features/recovery/recovery-screen"
import { RouteError, RouteNotFound } from "@/features/workspace/route-failures"

export const Route = createFileRoute("/recovery")({
  component: RecoveryScreen,
  errorComponent: RouteError,
  notFoundComponent: RouteNotFound,
})
