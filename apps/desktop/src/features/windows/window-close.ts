import { createStore } from "@tanstack/react-store"

import { flushAllDrafts } from "@/features/consoles/draft-registry"
import type { WindowCloseCost } from "@/features/consoles/use-consoles"
import { recovery } from "@/lib/ipc/recovery"
import { windows } from "@/lib/ipc/windows"

// The close of a window that is not the last (ADR-0043). The backend has had
// the window's transactions resolved before it asks (`closeRequested`); what
// is left is the consoles: one dialog lists those that would lose work, and
// confirmed, every console of every workspace of the window is closed before
// the backend is told the window may go. The backend then releases what the
// window still held — sessions, conversations, results — and destroys it.

/** What one workspace of the window offers its close. */
export interface WorkspaceConsoles {
  /** The connection's name, as the workspace shows it. */
  connection: string
  costs: () => Array<WindowCloseCost>
  closeAll: () => Promise<void>
}

const workspaces = new Map<string, WorkspaceConsoles>()

/** Called by each workspace while it is mounted, keyed by its session. */
export function registerWorkspaceConsoles(
  key: string,
  consoles: WorkspaceConsoles
) {
  workspaces.set(key, consoles)
  return () => {
    if (workspaces.get(key) === consoles) workspaces.delete(key)
  }
}

/** A console that would lose work, named with its workspace's connection. */
export interface WindowCloseRow extends WindowCloseCost {
  key: string
  connection: string
}

export interface WindowClose {
  rows: Array<WindowCloseRow>
  connections: Array<string>
  busy: boolean
}

/** `null` while no close of this window is being decided. */
export const windowClose = createStore<WindowClose | null>(null)

/** What closing the window now would cost, across its workspaces. */
export function windowCloseRows(
  held: ReadonlyMap<string, WorkspaceConsoles> = workspaces
): WindowClose {
  const rows: Array<WindowCloseRow> = []
  const connections: Array<string> = []
  for (const [workspace, consoles] of held) {
    if (!connections.includes(consoles.connection))
      connections.push(consoles.connection)
    consoles.costs().forEach((cost, index) =>
      rows.push({
        ...cost,
        key: `${workspace}:${index}`,
        connection: consoles.connection,
      })
    )
  }
  return { rows, connections, busy: false }
}

/**
 * `closeRequested`: acknowledged at once — past two seconds without it, the
 * backend takes this webview for frozen and closes the window without its
 * consoles — then the dialog, or the close itself when nothing would be lost.
 */
export function onCloseRequested() {
  void recovery.shutdownAcknowledged().catch(() => undefined)
  const asked = windowCloseRows()
  if (asked.rows.length === 0) {
    windowClose.setState(() => ({ ...asked, busy: true }))
    void closeWindow()
    return
  }
  windowClose.setState(() => asked)
}

/** The dialog's `Cancel`: the window stays, with every console as it was. */
export function cancelWindowClose() {
  windowClose.setState(() => null)
  void recovery.cancelExit().catch(() => undefined)
}

/**
 * Closes every console of the window, then tells the backend the window may
 * go. The drafts are flushed first: a closed console's text stays in the
 * library, as its autosave left it.
 */
export async function closeWindow() {
  const current = windowClose.state
  if (!current) return
  windowClose.setState(() => ({ ...current, busy: true }))
  await flushAllDrafts().catch(() => false)
  for (const consoles of [...workspaces.values()]) {
    await consoles.closeAll().catch(() => undefined)
  }
  await windows.confirmClose().catch(() => undefined)
}
