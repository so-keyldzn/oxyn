import { createStore } from "@tanstack/react-store"

import type { SaveStatus } from "@/components/oxyn/appearance-settings"
import type { BackendFailure } from "@/components/oxyn/backend-error-alert"
import { BackendError } from "@/lib/ipc/client"
import { settingsBackend } from "@/lib/ipc/settings"
import type {
  DisplayPreferences,
  PreferencesChange,
  PreferencesState,
} from "@/lib/ipc/settings"

/** Where the last save stands. A failure never reads as saved (UX-SPEC). */
export type SaveState = SaveStatus

export interface PreferencesStore {
  /** Applied locally: what the interface shows, saved or not yet. */
  preferences: DisplayPreferences
  /** False until the backend answered the first read. */
  loaded: boolean
  loadError: BackendFailure | null
  save: SaveState
}

/**
 * The domain defaults of `WorkspacePreferences`, shown only until the first
 * read answers. Never written: the backend numbers and merges every write.
 */
export const DEFAULT_PREFERENCES: DisplayPreferences = {
  theme: "dark",
  readingDensity: "compact",
  sidebarCollapsed: false,
  inspectorOpen: true,
  inspectorWidth: 280,
  nullText: "∅ NULL",
  groupThousands: false,
  binaryDisplay: "hex",
  cellMaxChars: 512,
}

export const preferencesStore = createStore<PreferencesStore>({
  preferences: DEFAULT_PREFERENCES,
  loaded: false,
  loadError: null,
  save: { status: "idle" },
})

function failureOf(error: unknown): BackendFailure {
  return error instanceof BackendError
    ? { message: error.message, retryable: error.retryable }
    : { message: String(error), retryable: false }
}

let loading: Promise<void> | null = null

/** Reads the preferences once per window load; later calls share the read. */
export function loadPreferences(): Promise<void> {
  loading ??= settingsBackend
    .readPreferences()
    .then((state: PreferencesState) => {
      preferencesStore.setState((current) => ({
        ...current,
        preferences: state.preferences,
        loaded: true,
        loadError: null,
      }))
    })
    .catch((error: unknown) => {
      loading = null
      preferencesStore.setState((current) => ({
        ...current,
        loadError: failureOf(error),
      }))
    })
  return loading
}

// Answers can arrive out of order: only the latest request settles the status.
let latest = 0

/**
 * Applies a change now, then saves it.
 *
 * Local first — a display setting is local (UX-SPEC « Ce qui n'est jamais
 * optimiste ») — and the save state says whether it reached the workspace.
 */
export async function changePreferences(change: PreferencesChange) {
  const request = ++latest
  preferencesStore.setState((current) => ({
    ...current,
    preferences: { ...current.preferences, ...change },
    save: { status: "saving" },
  }))
  try {
    await settingsBackend.writePreferences(change)
    if (request !== latest) return
    preferencesStore.setState((current) => ({
      ...current,
      save: { status: "saved" },
    }))
  } catch (error) {
    if (request !== latest) return
    preferencesStore.setState((current) => ({
      ...current,
      save: { status: "failed", message: failureOf(error).message },
    }))
  }
}

/** Saves what is applied again, under a new revision. */
export function retrySavingPreferences() {
  return changePreferences({})
}
