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
import { toast } from "@/components/ui/toast"
import { BackendError } from "@/lib/ipc/client"
import { recovery } from "@/lib/ipc/recovery"
import { windows } from "@/lib/ipc/windows"

function reportUnread(title: string, error: unknown) {
  toast.add({
    title,
    description: error instanceof BackendError ? error.message : String(error),
    type: "warning",
  })
}

export const Route = createFileRoute("/")({
  // Once per launch, per window. After an ordinary close, the window's
  // consoles come back offline, without a question; after an abnormal
  // shutdown the backend observed — or when how the last one ended cannot be
  // read —, the recovery screen offers them to choose from, even when there
  // are none. A window opened by `New window` has none (ADR-0021, ADR-0043).
  beforeLoad: async () => {
    if (session.state.recoveryOffered) return
    markRecoveryOffered()
    // Built by `Open in new window`: the tab it received opens at once, on
    // the connection it came with. Such a window has nothing to restore.
    // Taken once by the backend: an answer this build cannot read is said,
    // never swallowed — the session it carried is this window's until it
    // closes.
    const handoff = await windows.takeHandoff().catch((error: unknown) => {
      reportUnread("The moved tab could not be opened", error)
      return null
    })
    if (handoff) {
      if (handoff.type === "console") setMovedConsole(handoff)
      openConnection(handoff.open)
      throw redirect({ to: "/workspace" })
    }
    const [status, copies] = await Promise.allSettled([
      recovery.status(),
      windows.restoredConsoles(),
    ])
    if (status.status === "rejected")
      reportUnread("How Oxyn last closed could not be read", status.reason)
    if (copies.status === "rejected")
      reportUnread("This window's consoles could not be read", copies.reason)
    // The screen, not the connection list, whenever the outcome is not known
    // to be clean: an abnormal stop with no copy left still carries the
    // unknown-outcome warning (I-13), and an unread answer is no clean
    // shutdown — the backend reads an unreadable history the same way.
    if (
      status.status === "rejected" ||
      status.value.abnormal ||
      copies.status === "rejected"
    ) {
      // Unread, the screen lists every open copy instead of this window's.
      if (copies.status === "fulfilled") setLaunchCopies(copies.value)
      throw redirect({ to: "/recovery", search: { startup: true } })
    }
    if (copies.value.length > 0) restoreWorkingCopies(copies.value)
  },
  component: ConnectionScreen,
})
