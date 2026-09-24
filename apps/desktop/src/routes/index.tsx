import { createFileRoute, redirect } from "@tanstack/react-router"

import { ConnectionScreen } from "@/features/connections/connection-screen"
import { markRecoveryOffered, session } from "@/features/session"
import { newCommandId } from "@/lib/ipc/client"
import { library } from "@/lib/ipc/library"
import { recovery } from "@/lib/ipc/recovery"

export const Route = createFileRoute("/")({
  // Once per launch, and only after an abnormal shutdown the backend observed
  // with working copies to offer: a clean start speaks of no crash (ADR-0021).
  beforeLoad: async () => {
    if (session.state.recoveryOffered) return
    markRecoveryOffered()
    const status = await recovery.status().catch(() => ({ abnormal: false }))
    if (!status.abnormal) return
    const open = await library
      .listDocuments(newCommandId(), {
        savedOnly: false,
        openOnly: true,
        search: "",
        before: null,
        limit: 1,
      })
      .catch(() => null)
    if (open && open.entries.length > 0)
      throw redirect({ to: "/recovery", search: { startup: true } })
  },
  component: ConnectionScreen,
})
