// Mirror of `crates/oxyn-desktop/src/ipc/location.rs`, as executable schemas
// (ADR-0031). Both sides change in the same commit.

import { z } from "zod"

import { call } from "./client"
import { CatalogAddress } from "./types"

export const SectionChoice = z.enum([
  "data",
  "structure",
  "indexes",
  "constraints",
  /** Keys this relation declares towards others. */
  "relations",
  /** Keys other relations declare towards this one. */
  "incomingRelations",
  "definition",
])
export type SectionChoice = z.infer<typeof SectionChoice>

/** An object tab to restore: its address and the sub-view that was shown. */
export const ObjectPlace = z.object({
  address: CatalogAddress,
  section: SectionChoice,
})
export type ObjectPlace = z.infer<typeof ObjectPlace>

/**
 * The object tab saved last, with its connection: restored on that connection
 * only. `#[serde(flatten)]` on the Rust side, hence `extend`.
 */
export const SavedObjectPlace = ObjectPlace.extend({ connection: z.string() })
export type SavedObjectPlace = z.infer<typeof SavedObjectPlace>

export const location = {
  /** The tab saved last; `null` when none is. Reads no server. */
  readObjectLocation: () =>
    call("read_object_location", SavedObjectPlace.nullable()),

  /** Saves the tab shown, or forgets this connection's (`null`). */
  writeObjectLocation: (connection: string, place: ObjectPlace | null) =>
    call("write_object_location", z.null(), { connection, place }),
}
