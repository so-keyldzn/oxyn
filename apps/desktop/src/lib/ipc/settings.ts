// Mirror of `crates/oxyn-desktop/src/ipc/settings.rs`, as executable schemas:
// both sides still change in the same commit, but a field renamed on one of
// them is now named at the boundary instead of surfacing as an `undefined`
// (ADR-0031).

import { z } from "zod"

import { call } from "./client"
import {
  ApprovalPreview,
  Environment,
  PrivacyTier,
  SavedConnection,
} from "./types"

export const ThemeChoice = z.enum(["light", "dark", "system"])
export type ThemeChoice = z.infer<typeof ThemeChoice>

export const DensityChoice = z.enum(["compact", "comfortable"])
export type DensityChoice = z.infer<typeof DensityChoice>

export const BinaryChoice = z.enum(["hex", "base64", "size"])
export type BinaryChoice = z.infer<typeof BinaryChoice>

export const DisplayPreferences = z.object({
  theme: ThemeChoice,
  readingDensity: DensityChoice,
  sidebarCollapsed: z.boolean(),
  inspectorOpen: z.boolean(),
  /** Logical pixels, 240 to 480 (ADR-0013). */
  inspectorWidth: z.number().int().nonnegative(),
  /** What the grid draws for an absent value, at most 64 UTF-8 bytes. */
  nullText: z.string(),
  groupThousands: z.boolean(),
  binaryDisplay: BinaryChoice,
  /** 64 to 16 384: a cell is never uncut (I-06). */
  cellMaxChars: z.number().int().nonnegative(),
})
export type DisplayPreferences = z.infer<typeof DisplayPreferences>

/** Only the fields present change: several views write preferences. */
export const PreferencesChange = DisplayPreferences.partial()
export type PreferencesChange = z.infer<typeof PreferencesChange>

export const PreferencesState = z.object({
  revision: z.number().nonnegative(),
  preferences: DisplayPreferences,
})
export type PreferencesState = z.infer<typeof PreferencesState>

export const PreferencesSaved = z.object({
  /** The revision this write was given. */
  revision: z.number().nonnegative(),
  /** What is stored now; possibly a newer revision than this write's. */
  saved: PreferencesState,
})
export type PreferencesSaved = z.infer<typeof PreferencesSaved>

/**
 * A saved connection as lists show it. `location` tells homonyms apart with
 * declared, non-secret values only: host, port, database, file name.
 *
 * `#[serde(flatten)]` on the Rust side: the connection's own fields sit at the
 * same level as these two, which is why this extends rather than nests.
 */
export const ConnectionSummary = SavedConnection.extend({
  /** `PostgreSQL` rather than `postgres`. */
  driverName: z.string(),
  location: z.string().nullable(),
})
export type ConnectionSummary = z.infer<typeof ConnectionSummary>

/** Flattened like `ConnectionSummary`. */
export const ConnectionDetails = SavedConnection.extend({
  /** Non-secret parameters the driver declares. Never a secret. */
  values: z.record(z.string(), z.string()),
  /** Whether secrets are stored — never which, never their values. */
  hasStoredSecrets: z.boolean(),
})
export type ConnectionDetails = z.infer<typeof ConnectionDetails>

export const ConnectionEdit = z.object({
  name: z.string(),
  environment: Environment,
  privacyTier: PrivacyTier,
  readOnly: z.boolean(),
  values: z.record(z.string(), z.string()),
  /** Only the secrets the user retyped; an absent key keeps the stored one. */
  secrets: z.record(z.string(), z.string()),
})
export type ConnectionEdit = z.infer<typeof ConnectionEdit>

export const ConnectionChange = z.discriminatedUnion("type", [
  z.object({
    type: z.literal("saved"),
    connection: SavedConnection,
    /** The configuration was saved but the retyped secrets were not. */
    secretsError: z.string().nullable(),
  }),
  z.object({ type: z.literal("deleted") }),
  z.object({
    type: z.literal("approval"),
    command: z.string(),
    reason: z.string(),
    preview: ApprovalPreview.nullable(),
  }),
])
export type ConnectionChange = z.infer<typeof ConnectionChange>

export const settingsBackend = {
  /**
   * Listed here rather than in `client.ts`, where it used to sit: `call` would
   * have had to import this file's `ConnectionSummary` while this file imports
   * `call`, and with schemas being values that cycle is no longer erased at
   * compile time. It happened to work only because the schema was read inside
   * a lambda; hoisting it out of one would have made it `undefined` at load.
   */
  listConnections: () => call("list_connections", z.array(ConnectionSummary)),

  readPreferences: () => call("read_preferences", PreferencesState),

  writePreferences: (change: PreferencesChange) =>
    call("write_preferences", PreferencesSaved, { change }),

  connectionDetails: (connection: string) =>
    call("connection_details", ConnectionDetails, { connection }),

  /**
   * `null` when a change of environment or privacy tier was not confirmed in
   * the host's native dialog: nothing of the edit is saved (ADR-0037).
   */
  updateConnection: (
    commandId: string,
    connection: string,
    edit: ConnectionEdit
  ) =>
    call("update_connection", ConnectionChange.nullable(), {
      commandId,
      connection,
      edit,
    }),

  deleteConnection: (commandId: string, connection: string) =>
    call("delete_connection", ConnectionChange, { commandId, connection }),

  decideConnectionChange: (command: string, approved: boolean) =>
    call("decide_connection_change", ConnectionChange.nullable(), {
      command,
      approved,
    }),

  /** The marking as saved now, to merge into an open connection's copy (I-04). */
  connectionMarking: (connection: string) =>
    call("connection_marking", SavedConnection, { connection }),
}
