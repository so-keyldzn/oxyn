import { createFileRoute } from "@tanstack/react-router"
import { z } from "zod"

import { RecoveryScreen } from "@/features/recovery/recovery-screen"
import { RouteError, RouteNotFound } from "@/features/workspace/route-failures"

// `startup` is set by the automatic opening after launch only. Opened on
// demand, the screen must not announce a crash that happened, if at all,
// before the session the user is in (ADR-0021).
const RecoverySearch = z.object({
  startup: z.boolean().optional().catch(undefined),
})

export const Route = createFileRoute("/recovery")({
  validateSearch: RecoverySearch,
  component: RecoveryScreen,
  errorComponent: RouteError,
  notFoundComponent: RouteNotFound,
})
