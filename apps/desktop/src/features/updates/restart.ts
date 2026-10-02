import { toast } from "@/components/ui/toast"
import { updates } from "@/lib/ipc/updates"

import { message, patchUpdates } from "./update-store"

// `Restart now` (ADR-0051, docs/UX-SPEC.md « Updates »): the ordered quit of
// ⌘Q, then a relaunch. Asked unconfirmed first — with nothing running, it
// does not ask; running queries or exports are named before anything stops.
// An open transaction then holds it like the exit, in the `restart` scope
// (`features/recovery/exit-transactions.ts`).

export function restartNow() {
  void ask(false)
}

/** `Stop and restart`: the user chose to stop the work the dialog named. */
export function stopAndRestart() {
  patchUpdates({ restartWork: null })
  void ask(true)
}

/** `Later`: nothing stopped, the update still installs at the next quit. */
export function postponeRestart() {
  patchUpdates({ restartWork: null })
}

async function ask(confirmed: boolean) {
  try {
    const outcome = await updates.restartToUpdate({ confirmed })
    if (outcome.type === "busy")
      patchUpdates({
        restartWork: { running: outcome.running, exports: outcome.exports },
      })
  } catch (error) {
    toast.add({
      title: "Oxyn could not restart",
      description: message(error),
      type: "error",
    })
  }
}
