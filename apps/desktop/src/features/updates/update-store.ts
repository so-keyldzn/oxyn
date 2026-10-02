import { createStore } from "@tanstack/react-store"

import { toast } from "@/components/ui/toast"
import type {
  JustUpdated,
  UpdatePreferenceSave,
} from "@/components/oxyn/update-settings"
import { BackendError } from "@/lib/ipc/client"
import { updates } from "@/lib/ipc/updates"
import type { UpdateSnapshot } from "@/lib/ipc/updates"

// The webview's copy of where the update stands (ADR-0051). Rust owns the
// state and pushes every change on `subscribe_updates`; this store only holds
// the last snapshot, and what the front alone knows: a failed save of the
// switch, the notice of this launch, the restart waiting for a confirmation.

export interface UpdatesView {
  /** `null` until the backend answered once. */
  snapshot: UpdateSnapshot | null
  save: UpdatePreferenceSave
  justUpdated: JustUpdated | null
  /** Work `Restart now` would stop: the confirmation is open. */
  restartWork: { running: number; exports: number } | null
  /** `Check for updates…` opened the section: its heading takes the focus. */
  focusRequested: boolean
}

export const updateStore = createStore<UpdatesView>({
  snapshot: null,
  save: { status: "idle" },
  justUpdated: null,
  restartWork: null,
  focusRequested: false,
})

export function patchUpdates(patch: Partial<UpdatesView>) {
  updateStore.setState((current) => ({ ...current, ...patch }))
}

function receive(snapshot: UpdateSnapshot) {
  patchUpdates({ snapshot })
}

let subscribed = false

/** Once per webview: the current state, then every change. */
export function subscribeToUpdates() {
  if (subscribed) return
  subscribed = true
  // Subscribed first: a change between the read and the subscription would
  // otherwise be lost until the next one.
  void updates.subscribe(receive).catch((error: unknown) => {
    subscribed = false
    console.error(`Could not follow the updates: ${message(error)}`)
  })
  void updates
    .state()
    .then(receive)
    .catch((error: unknown) => {
      console.error(`Could not read the update state: ${message(error)}`)
    })
}

export function message(error: unknown) {
  return error instanceof BackendError ? error.message : String(error)
}

/**
 * A request the backend refused: said once, in a toast. A failure of the
 * operation itself arrives as a state, and Settings shows it.
 */
function refused(what: string) {
  return (error: unknown) => {
    toast.add({ title: what, description: message(error), type: "error" })
  }
}

export function checkForUpdates() {
  void updates.check().catch(refused("Could not check for updates"))
}

export function cancelUpdate() {
  void updates.cancel().catch(refused("Could not stop the update"))
}

export function downloadUpdate() {
  void updates.download().catch(refused("Could not download the update"))
}

export function openReleasePage() {
  void updates
    .openReleasePage()
    .catch(refused("Could not open the release page"))
}

/**
 * A failed save leaves the setting applied for the session: Rust holds it in
 * memory, and the snapshot says so. The view offers `Save again`.
 */
export async function setAutomaticUpdates(automatic: boolean) {
  try {
    await updates.setAutomatic(automatic)
    patchUpdates({ save: { status: "idle" } })
  } catch (error) {
    patchUpdates({ save: { status: "failed", message: message(error) } })
  }
}

export function retrySavingAutomatic() {
  const automatic = updateStore.state.snapshot?.automatic
  if (automatic !== undefined) void setAutomaticUpdates(automatic)
}
