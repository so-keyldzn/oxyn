// Mirror of `crates/oxyn-desktop/src/ipc/completion.rs`, as executable schemas
// (ADR-0057). Every name here is the server's word, hence hostile data: it is
// displayed as text, and the editor inserts the `insert` beside it — never a
// form it composed itself.

import { z } from "zod"

import { call } from "./client"
import type { NamePart } from "@/lib/sql-identifiers"

export const CompletionField = z.object({
  name: z.string(),
  insert: z.string(),
  /** The type as the server spells it. */
  dataType: z.string(),
})
export type CompletionField = z.infer<typeof CompletionField>

/**
 * Three answers, not two: a relation nobody described and one whose columns
 * are all loaded must not look alike, nor one whose columns a bound cut.
 */
export const CompletionFields = z.discriminatedUnion("state", [
  z.object({ state: z.literal("notLoaded") }),
  z.object({ state: z.literal("loaded"), items: z.array(CompletionField) }),
  z.object({
    state: z.literal("omitted"),
    count: z.number().int().nonnegative(),
  }),
])
export type CompletionFields = z.infer<typeof CompletionFields>

export const CompletionRelation = z.object({
  name: z.string(),
  /** The form to insert for the name alone. */
  insert: z.string(),
  /** The form to insert when the namespace is part of it. */
  qualified: z.string(),
  kind: z.string(),
  /** Described, then invalidated by a DDL: still suggested, read again. */
  stale: z.boolean(),
  virtualTable: z.boolean(),
  shadow: z.boolean(),
  fields: CompletionFields,
})
export type CompletionRelation = z.infer<typeof CompletionRelation>

export const CompletionNamespace = z.object({
  /** `null` for relations at the root of a source with no namespaces. */
  name: z.string().nullable(),
  insert: z.string().nullable(),
  /** Suggested only after its qualifier. */
  system: z.boolean(),
  stale: z.boolean(),
  /** `false`: its relations were never listed, which is not « none ». */
  listed: z.boolean(),
  relations: z.array(CompletionRelation),
})
export type CompletionNamespace = z.infer<typeof CompletionNamespace>

/** Where an unqualified name resolves. */
export const CompletionResolution = z.discriminatedUnion("state", [
  /**
   * Never read, the source cannot say, or the path is not one the snapshot can
   * carry whole (too many entries, an entry that hides text): nothing is
   * suggested unqualified.
   */
  z.object({ state: z.literal("unknown") }),
  z.object({
    state: z.literal("known"),
    /** First match first. May be empty: nothing resolves, and that is known. */
    namespaces: z.array(z.string()),
    stale: z.boolean(),
  }),
])
export type CompletionResolution = z.infer<typeof CompletionResolution>

export const CompletionTruncation = z.object({
  /** Counted even when the namespace holds nothing in the cache. */
  namespacesLeftOut: z.number().int().nonnegative(),
  relationsLeftOut: z.number().int().nonnegative(),
  fieldsLeftOut: z.number().int().nonnegative(),
})
export type CompletionTruncation = z.infer<typeof CompletionTruncation>

export const CompletionCatalog = z.object({
  namespaces: z.array(CompletionNamespace),
  resolution: CompletionResolution,
  /** Present when a bound cut the answer: a partial answer says so. */
  truncated: CompletionTruncation.nullable(),
})
export type CompletionCatalog = z.infer<typeof CompletionCatalog>

/** What one pass of the editor read — counts only. */
export const CompletionLoad = z.object({
  /** Reads submitted, successful or not. Zero: nothing was missing. */
  attempted: z.number().int().nonnegative(),
  /** Reads that failed or were refused. */
  failed: z.number().int().nonnegative(),
  /** Why the pass ended before it was done: `deadline` or `cancelled`. */
  stopped: z.string().nullable(),
})
export type CompletionLoad = z.infer<typeof CompletionLoad>

export const completion = {
  /**
   * What the cache of a connection holds, for the editor's completion. A cache
   * read: it asks no server, and the caller reads it when a signal says the
   * cache changed — never on a keystroke.
   */
  catalog: (connection: string) =>
    call("completion_catalog", CompletionCatalog, { connection }),

  /**
   * Has the host read, through the bus, what the cache lacks for the names of
   * the statement under the cursor — bounded, and replaced by the next call
   * for the same connection. Resolves when the pass is over; what it read
   * reaches the snapshot through the refresh signals, not through this answer.
   *
   * The names are as the editor lexed them: the host compares them with the
   * cache's and never composes them into a read (I-10). After a pause in
   * typing, never on a keystroke.
   */
  load: (connection: string, names: Array<Array<NamePart>>) =>
    call("completion_load", CompletionLoad, { connection, names }),
}
