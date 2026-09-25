import { createFileRoute, redirect } from "@tanstack/react-router"

import { ConnectionScreen } from "@/features/connections/connection-screen"
import {
  markRecoveryOffered,
  openConnection,
  restoreWorkingCopies,
  session,
  setLaunchCopies,
  setMovedConsole,
} from "@/features/session"
import { recovery } from "@/lib/ipc/recovery"
import { windows } from "@/lib/ipc/windows"

export const Route = createFileRoute("/")({
  // Once per launch, per window. After an ordinary close, the window's
  // consoles come back offline, without a question; after an abnormal
  // shutdown the backend observed, the recovery screen offers them to choose
  // from. A window opened by `New window` has none (ADR-0021, ADR-0043).
  beforeLoad: async () => {
    if (session.state.recoveryOffered) return
    markRecoveryOffered()
    // Built by `Open in new window`: the tab it received opens at once, on
    // the connection it came with. Such a window has nothing to restore.
    const handoff = await windows.takeHandoff().catch(() => null)
    if (handoff) {
      if (handoff.type === "console") setMovedConsole(handoff)
      openConnection(handoff.open)
      throw redirect({ to: "/workspace" })
    }
    const [status, copies] = await Promise.all([
      recovery.status().catch(() => ({ abnormal: false })),
      windows.restoredConsoles().catch(() => []),
    ])
    if (copies.length === 0) return
    if (status.abnormal) {
      setLaunchCopies(copies)
      throw redirect({ to: "/recovery", search: { startup: true } })
    }
    restoreWorkingCopies(copies)
  },
  component: ConnectionScreen,
})
