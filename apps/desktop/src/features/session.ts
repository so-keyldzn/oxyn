import { createStore } from "@tanstack/react-store"

import type { DocumentEntry } from "@/lib/ipc/library"
import type { OpenConnection } from "@/lib/ipc/types"

/**
 * Workspaces a window keeps connected at once (ADR-0046). A guard, not a
 * measure: each holds at least two server sessions and a catalog cache.
 */
export const MAX_RETAINED_WORKSPACES = 8

export interface SessionState {
  /** The connection whose workspace is shown, once the user opened one. */
  open: OpenConnection | null
  /**
   * Every connection with a workspace in this window, shown or not, in the
   * order they were opened. Each keeps its sessions until an explicit
   * disconnect (ADR-0046).
   */
  workspaces: Array<OpenConnection>
  /**
   * The SQL draft. Copied from the shown workspace into the first console of
   * the next new connection, without being run (docs/UX-SPEC.md,
   * « Navigation du premier workspace »).
   */
  sqlDraft: string
  /** The connection the draft was copied from, named when the copy lands. */
  sqlDraftFrom: string | null
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
  workspaces: [],
  sqlDraft: "SELECT 1;",
  sqlDraftFrom: null,
  restored: [],
  recoveryOffered: false,
  objectPlaceDeclined: false,
})

// The Rust sessions outlive a reload of the webview; this store does not.
// Only the connection ids are kept, for this webview only, so that a reload
// can close what it can no longer show (docs/UX-SPEC.md).
const RELOAD_KEY = "oxyn.open-connections"

/**
 * Connections disconnected whose sessions are not closed yet: the host writes
 * their drafts first. A reload meanwhile must still close them.
 */
const leaving = new Set<string>()

function rememberForReload(workspaces: Array<OpenConnection>) {
  const connections = new Set([
    ...workspaces.map((open) => open.connection),
    ...leaving,
  ])
  try {
    if (connections.size === 0) sessionStorage.removeItem(RELOAD_KEY)
    else sessionStorage.setItem(RELOAD_KEY, JSON.stringify([...connections]))
  } catch {
    // No storage (private window, tests): a reload then leaves the sessions
    // to the backend's shutdown.
  }
}

/** The connections a previous load of this webview left open, once. */
export function takeConnectionsLeftByReload(): Array<string> {
  if (session.state.workspaces.length > 0) return []
  try {
    const stored = sessionStorage.getItem(RELOAD_KEY)
    sessionStorage.removeItem(RELOAD_KEY)
    const parsed: unknown = stored === null ? [] : JSON.parse(stored)
    return Array.isArray(parsed)
      ? parsed.filter((item): item is string => typeof item === "string")
      : []
  } catch {
    return []
  }
}

function withWorkspaces(
  state: SessionState,
  workspaces: Array<OpenConnection>,
  open: OpenConnection | null
): SessionState {
  rememberForReload(workspaces)
  return { ...state, workspaces, open }
}

/**
 * Whether another connection may get a workspace in this window. One that
 * already has one is shown, never opened twice.
 */
export function canOpenWorkspace(connection?: string) {
  const { workspaces } = session.state
  return (
    workspaces.some((open) => open.connection === connection) ||
    workspaces.length < MAX_RETAINED_WORKSPACES
  )
}

/**
 * Shows the workspace of `open`, adding it when its connection had none. The
 * same connection handed back — new settings, or a session reopened —
 * replaces its entry in place.
 */
export function openConnection(open: OpenConnection) {
  session.setState((state) => {
    const known = state.workspaces.some(
      (held) => held.connection === open.connection
    )
    const workspaces = known
      ? state.workspaces.map((held) =>
          held.connection === open.connection ? open : held
        )
      : [...state.workspaces, open]
    return withWorkspaces(state, workspaces, open)
  })
}

/**
 * Hands a retained workspace its connection's new settings — marking, tier —
 * without changing which workspace is shown: an edit made in Settings reaches
 * a hidden workspace too (I-04).
 */
export function refreshConnection(open: OpenConnection) {
  session.setState((state) => {
    if (!state.workspaces.some((held) => held.session === open.session))
      return state
    return withWorkspaces(
      state,
      state.workspaces.map((held) =>
        held.session === open.session ? open : held
      ),
      state.open?.session === open.session ? open : state.open
    )
  })
}

/**
 * Shows the workspace a connection already has; `false` when it has none.
 * Nothing is sent to the backend: its sessions never closed (ADR-0046).
 */
export function showConnection(connection: string) {
  const held = session.state.workspaces.find(
    (open) => open.connection === connection
  )
  if (!held) return false
  session.setState((state) => ({ ...state, open: held }))
  return true
}

/**
 * Ends a connection's workspace — the shown one by default. The host then
 * writes its drafts and closes its sessions.
 */
export function closeConnection(
  connection: string | undefined = session.state.open?.connection
) {
  if (connection !== undefined) leaving.add(connection)
  session.setState((state) =>
    withWorkspaces(
      state,
      state.workspaces.filter((open) => open.connection !== connection),
      state.open?.connection === connection ? null : state.open
    )
  )
}

/** The host closed what a disconnected workspace held: a reload may forget it. */
export function connectionReleased(connection: string) {
  if (!leaving.delete(connection)) return
  rememberForReload(session.state.workspaces)
}

export function setSqlDraft(sqlDraft: string, sqlDraftFrom: string | null) {
  session.setState((state) => ({ ...state, sqlDraft, sqlDraftFrom }))
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
