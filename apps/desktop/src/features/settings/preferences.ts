import { createStore } from "@tanstack/react-store"

import type { SaveStatus } from "@/components/oxyn/appearance-settings"
import type { BackendFailure } from "@/components/oxyn/backend-error-alert"
import { toast } from "@/components/ui/toast"
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
  /**
   * Counts the saves that changed how the backend formats cells. Result pages
   * are formatted there and cached until invalidated: a new value says the
   * cached ones are formatted the old way.
   */
  formatRevision: number
}

/** The preferences the backend formats cells with (`format_options_of`). */
const FORMAT_FIELDS = [
  "nullText",
  "groupThousands",
  "binaryDisplay",
  "cellMaxChars",
] as const satisfies ReadonlyArray<keyof DisplayPreferences>

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
  formatRevision: 0,
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

/** How one save ended; `superseded` when a later change answers instead. */
export type SaveOutcome =
  | { status: "saved" }
  | { status: "failed"; message: string }
  | { status: "superseded" }

/**
 * Applies a change now, then saves it.
 *
 * Local first — a display setting is local (UX-SPEC « Ce qui n'est jamais
 * optimiste ») — and the save state says whether it reached the workspace.
 */
export async function changePreferences(
  change: PreferencesChange
): Promise<SaveOutcome> {
  const request = ++latest
  preferencesStore.setState((current) => ({
    ...current,
    preferences: { ...current.preferences, ...change },
    save: { status: "saving" },
  }))
  try {
    await settingsBackend.writePreferences(change)
    // Before the superseded check: this write reached the backend whatever
    // answers after it, so pages formatted before it are stale.
    if (FORMAT_FIELDS.some((field) => field in change))
      preferencesStore.setState((current) => ({
        ...current,
        formatRevision: current.formatRevision + 1,
      }))
    if (request !== latest) return { status: "superseded" }
    preferencesStore.setState((current) => ({
      ...current,
      save: { status: "saved" },
    }))
    return { status: "saved" }
  } catch (error) {
    if (request !== latest) return { status: "superseded" }
    const message = failureOf(error).message
    preferencesStore.setState((current) => ({
      ...current,
      save: { status: "failed", message },
    }))
    return { status: "failed", message }
  }
}

/** Saves what is applied again, under a new revision. */
export function retrySavingPreferences() {
  return changePreferences({})
}

/**
 * A change made from the workspace rather than from the settings dialog.
 *
 * The failure is treated as the dialog treats it — applied, said not saved,
 * with `Save again` — but in a toast, since no save status line is on screen
 * there. A failure never reads as saved (ADR-0013).
 */
export async function changePreferencesFromView(change: PreferencesChange) {
  const outcome = await changePreferences(change)
  if (outcome.status !== "failed") return
  toast.add({
    title: "Preference applied here but not saved",
    description: outcome.message,
    type: "warning",
    actionProps: {
      children: "Save again",
      onClick: () => void changePreferencesFromView({}),
    },
  })
}
