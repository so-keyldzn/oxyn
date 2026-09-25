import { refreshConnection, session } from "@/features/session"
import { reloadPreferences } from "@/features/settings/preferences"
import { onCloseRequested } from "@/features/windows/window-close"
import { settingsBackend } from "@/lib/ipc/settings"
import { windows } from "@/lib/ipc/windows"

// What the backend tells this window alone, on its own channel (ADR-0043):
// its close was asked, or something another window changed must be read
// again. A notice, never data: what is shown is read again through the bus.

/**
 * A saved connection this window holds was edited, here or elsewhere: every
 * workspace of it takes the new marking, shown or hidden. A stale tier would
 * let the assistant answer under a level the connection no longer has (I-04).
 */
export async function rereadConnection(connection: string) {
  const held = session.state.workspaces.filter(
    (open) => open.connection === connection
  )
  if (held.length === 0) return
  const marking = await settingsBackend
    .connectionMarking(connection)
    .catch(() => null)
  if (!marking) return
  for (const open of held)
    refreshConnection({
      ...open,
      name: marking.name,
      environment: marking.environment,
      readOnly: marking.readOnly,
      privacyTier: marking.privacyTier,
    })
}

let subscribed = false

/** Called once by the root, like the other channels of the window. */
export function subscribeToWindowSignals() {
  if (subscribed) return
  subscribed = true
  windows
    .subscribe((signal) => {
      switch (signal.type) {
        case "closeRequested":
          onCloseRequested()
          return
        case "connectionChanged":
          void rereadConnection(signal.connection)
          return
        case "preferencesChanged":
          void reloadPreferences()
      }
    })
    .catch(() => {
      // Outside the desktop application there is no window to close.
      subscribed = false
    })
}
