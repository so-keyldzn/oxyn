import * as React from "react"
import { useStore } from "@tanstack/react-store"

import { RestartToUpdateDialog } from "@/components/oxyn/restart-to-update-dialog"
import {
  checkAvailability,
  readyToastTitle,
} from "@/components/oxyn/update-model"
import { toast } from "@/components/ui/toast"
import { openSettings } from "@/features/settings/settings-dialog"
import { useActionSource } from "@/lib/actions/context"
import { updates } from "@/lib/ipc/updates"
import type { UpdateNotice } from "@/lib/ipc/updates"

import { postponeRestart, restartNow, stopAndRestart } from "./restart"
import {
  checkForUpdates,
  message,
  openReleasePage,
  patchUpdates,
  updateStore,
} from "./update-store"

/** Versions this webview already announced as ready: never twice. */
const announced = new Set<string>()

/**
 * Once per window, after `ExitTransactionsHost` (ADR-0051): `Check for
 * updates…` for the menu, the toasts said once each, and the confirmation of
 * `Restart now`. The lasting form of every toast is in Settings ▸ Updates.
 */
export function UpdateHost() {
  const state = useStore(updateStore, (view) => view.snapshot?.state ?? null)
  const restartWork = useStore(updateStore, (view) => view.restartWork)

  const availability = state ? checkAvailability(state) : null
  useActionSource(
    "updates",
    availability === null
      ? null
      : { off: availability.kind === "off" ? availability.reason : null },
    {
      checkForUpdates: () => {
        // The result lives in Settings ▸ Updates: a toast could hold neither
        // a failure's message nor `Cancel`.
        patchUpdates({ focusRequested: true })
        openSettings("updates")
        const current = updateStore.state.snapshot?.state
        if (current && checkAvailability(current).kind === "check")
          checkForUpdates()
      },
    }
  )

  // What the previous launch's update left to say: read once, by the first
  // window that asks.
  React.useEffect(() => {
    updates
      .takeNotice()
      .then((notice) => {
        if (notice) announceNotice(notice)
      })
      .catch((error: unknown) => {
        console.error(`Could not read the update notice: ${message(error)}`)
      })
  }, [])

  // The first time a version is downloaded, in the window the user is in. A
  // state already ready when this window first saw it was announced elsewhere.
  const seen = React.useRef(false)
  React.useEffect(() => {
    if (state === null) return
    const first = !seen.current
    seen.current = true
    if (state.type !== "ready" || announced.has(state.version)) return
    announced.add(state.version)
    if (first || !document.hasFocus()) return
    toast.add({
      title: readyToastTitle(state.version),
      type: "info",
      actionProps: { children: "Restart now", onClick: restartNow },
    })
  }, [state])

  return (
    <RestartToUpdateDialog
      work={restartWork}
      onLater={postponeRestart}
      onStopAndRestart={stopAndRestart}
    />
  )
}

function announceNotice(notice: UpdateNotice) {
  switch (notice.type) {
    case "installed":
      patchUpdates({
        justUpdated: { from: notice.from, at: new Date().toISOString() },
      })
      toast.add({
        title: `Oxyn updated to ${notice.to}`,
        description: `Updated from ${notice.from}.`,
        type: "info",
        actionProps: { children: "What's new", onClick: openReleasePage },
      })
      break
    case "installFailed":
      toast.add({
        title: `Oxyn ${notice.version} could not be installed`,
        description: notice.message,
        type: "error",
        actionProps: {
          children: "Update settings…",
          onClick: () => openSettings("updates"),
        },
      })
      break
  }
}
