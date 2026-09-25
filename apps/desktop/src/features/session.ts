import { createStore } from "@tanstack/react-store"

import type { DocumentEntry } from "@/lib/ipc/library"
import type { OpenConnection } from "@/lib/ipc/types"

export interface SessionState {
  /** The connection the workspace runs against, once the user opened one. */
  open: OpenConnection | null
  /**
   * The SQL draft. Kept across a connection switch and moved into the new
   * editor without being run (docs/UX-SPEC.md, « Navigation du premier
   * workspace »).
   */
  sqlDraft: string
  /**
   * Working copies chosen on the recovery screen, waiting for the shown
   * workspace, which reopens them as **offline** consoles: nothing connects
   * and nothing runs until the user attaches one (ADR-0021, UX-SPEC
   * § Restauration sélective au démarrage).
   */
  restored: Array<DocumentEntry>
  /** The recovery screen was offered once this launch; never twice. */
  recoveryOffered: boolean
  /**
   * The saved object tab was not chosen on the recovery screen: no workspace
   * reopens it this launch. It stays saved — declining erases nothing.
   */
  objectPlaceDeclined: boolean
}

export const session = createStore<SessionState>({
  open: null,
  sqlDraft: "SELECT 1;",
  restored: [],
  recoveryOffered: false,
  objectPlaceDeclined: false,
})

// The Rust sessions outlive a reload of the webview; this store does not.
// Only the connection id is kept, for this webview only, so that a reload can
// close what it can no longer show (docs/UX-SPEC.md).
const RELOAD_KEY = "oxyn.open-connection"

function rememberForReload(connection: string | null) {
  try {
    if (connection === null) sessionStorage.removeItem(RELOAD_KEY)
    else sessionStorage.setItem(RELOAD_KEY, connection)
  } catch {
    // No storage (private window, tests): a reload then leaves the sessions
    // to the backend's shutdown.
  }
}

/** The connection a previous load of this webview left open, once. */
export function takeConnectionLeftByReload(): string | null {
  if (session.state.open) return null
  try {
    const connection = sessionStorage.getItem(RELOAD_KEY)
    sessionStorage.removeItem(RELOAD_KEY)
    return connection
  } catch {
    return null
  }
}

export function openConnection(open: OpenConnection) {
  rememberForReload(open.connection)
  session.setState((state) => ({ ...state, open }))
}

export function closeConnection() {
  rememberForReload(null)
  session.setState((state) => ({ ...state, open: null }))
}

export function setSqlDraft(sqlDraft: string) {
  session.setState((state) => ({ ...state, sqlDraft }))
}

/**
 * Adds working copies to those waiting for a workspace. A new selection never
 * replaces one not yet taken, and a copy already waiting is not added twice.
 */
export function restoreWorkingCopies(restored: Array<DocumentEntry>) {
  session.setState((state) => ({
    ...state,
    restored: [
      ...state.restored,
      ...restored.filter(
        (entry) => !state.restored.some((waiting) => waiting.id === entry.id)
      ),
    ],
  }))
}

/** Hands the pending working copies to one workspace, once. */
export function takeRestoredWorkingCopies() {
  const restored = session.state.restored
  if (restored.length > 0)
    session.setState((state) => ({ ...state, restored: [] }))
  return restored
}

export function declineObjectPlace(declined: boolean) {
  session.setState((state) => ({ ...state, objectPlaceDeclined: declined }))
}

export function markRecoveryOffered() {
  session.setState((state) => ({ ...state, recoveryOffered: true }))
}

/** Whether the session declares a capability, by its `oxyn-core` name. */
export function hasCapability(open: OpenConnection, name: string) {
  return open.capabilities.includes(name)
}
