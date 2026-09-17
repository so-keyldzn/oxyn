// Mirror of `crates/oxyn-desktop/src/ipc/metadata.rs`, as executable schemas:
// both change in the same commit, and a field renamed on one side is now named
// by the validator on the first call instead of read as `undefined` (ADR-0031).

import { Channel, isTauri } from "@tauri-apps/api/core"
import { z } from "zod"

import { call, guarded, Nothing } from "./client"
import { ParameterKind } from "./consoles"
import {
  CatalogAddress,
  CatalogNode,
  CommandOutcome,
  RelationDetail,
} from "./types"

/** Rows one preview reads (`PREVIEW_ROWS`). */
export const PREVIEW_ROWS = 200

/** Sort keys one preview may carry (`MAX_SORT_KEYS`). */
export const MAX_SORT_KEYS = 16

/** A column name exactly as the catalog spells it; never an expression. */
export const PreviewSortKey = z.object({
  column: z.string(),
  descending: z.boolean(),
})
export type PreviewSortKey = z.infer<typeof PreviewSortKey>

/**
 * What a preview asks for. The predicate is SQL the user wrote, sent as is;
 * the sort is structured, and the backend rebuilds and checks both (ADR-0020).
 */
export const PreviewShape = z.object({
  sort: z.array(PreviewSortKey),
  predicate: z.string().nullable(),
  offset: z.number().int().nonnegative(),
})
export type PreviewShape = z.infer<typeof PreviewShape>

export const PLAIN_SHAPE: PreviewShape = {
  sort: [],
  predicate: null,
  offset: 0,
}

/** Which page controls the rows on screen allow (ADR-0028). */
export const Pagination = z.discriminatedUnion("type", [
  z.object({ type: z.literal("needsOrder") }),
  z.object({ type: z.literal("noUniqueKey") }),
  z.object({
    type: z.literal("ready"),
    previous: z.boolean(),
    next: z.boolean(),
    firstRow: z.number().int().nonnegative(),
  }),
])
export type Pagination = z.infer<typeof Pagination>

// Tagged by `state`, not by `type`: the Rust enum says so, and the two other
// unions of this file say `type`.
export const FacetFreshness = z.discriminatedUnion("state", [
  z.object({ state: z.literal("never") }),
  z.object({ state: z.literal("fetched"), fetchedAt: z.string() }),
  z.object({ state: z.literal("invalidated") }),
])
export type FacetFreshness = z.infer<typeof FacetFreshness>

/**
 * `value` is null when never read; `freshness` tells that from « none ».
 *
 * A schema taking a schema is a function, and `z.infer` cannot follow one: the
 * type is spelled out beside it, as `CatalogNode` is in `types.ts`.
 */
export function Facet<T extends z.ZodType>(value: T) {
  return z.object({ freshness: FacetFreshness, value: value.nullable() })
}
export type Facet<T> = { freshness: FacetFreshness; value: T | null }

export const ConstraintRow = z.object({
  name: z.string(),
  kind: z.string(),
  fields: z.array(z.string()),
  expression: z.string().nullable(),
  validated: z.boolean().nullable(),
})
export type ConstraintRow = z.infer<typeof ConstraintRow>

export const IndexRow = z.object({
  name: z.string(),
  fields: z.array(z.string()),
  unique: z.boolean(),
  method: z.string().nullable(),
  predicate: z.string().nullable(),
})
export type IndexRow = z.infer<typeof IndexRow>

export const ForeignKeyRow = z.object({
  name: z.string(),
  fields: z.array(z.string()),
  references: CatalogAddress,
  referencedFields: z.array(z.string()),
  onDelete: z.string(),
  cascades: z.boolean(),
  wellFormed: z.boolean(),
})
export type ForeignKeyRow = z.infer<typeof ForeignKeyRow>

export const IncomingKeyRow = z.object({
  source: CatalogAddress,
  key: ForeignKeyRow,
  sourceUnique: z.boolean().nullable(),
})
export type IncomingKeyRow = z.infer<typeof IncomingKeyRow>

export const DefinitionView = z.object({
  sql: z.string(),
  /**
   * `stored` or `reconstructed`: provenance stays visible (ADR-0018).
   *
   * A plain `String` in `ipc.rs`. The cases are named here because the tab
   * labels them, but a value this version does not know falls back to
   * `unknown` instead of rejecting the answer: a provenance added later should
   * cost one badge, not the whole definition (front.md).
   */
  source: z.enum(["stored", "reconstructed", "unknown"]).catch("unknown"),
  notes: z.array(z.string()),
})
export type DefinitionView = z.infer<typeof DefinitionView>

export const RelationFacets = z.object({
  address: CatalogAddress,
  kind: z.string().nullable(),
  holdsRecords: z.boolean(),
  /** Quoted by the session's dialect in Rust; copied as is, never rebuilt. */
  qualifiedName: z.string(),
  detail: Facet(RelationDetail),
  indexes: z.array(IndexRow).nullable(),
  foreignKeys: z.array(ForeignKeyRow).nullable(),
  constraints: Facet(z.array(ConstraintRow)),
  incomingKeys: Facet(z.array(IncomingKeyRow)),
  definition: Facet(DefinitionView),
  uniqueKey: z.boolean().nullable(),
})
export type RelationFacets = z.infer<typeof RelationFacets>

export const RelationFacet = z.enum([
  "detail",
  "constraints",
  "incomingKeys",
  "definition",
])
export type RelationFacet = z.infer<typeof RelationFacet>

/** The row a related-row query takes its values from. */
export const RelatedRowsSource = z.object({
  result: z.string(),
  row: z.number().int().nonnegative(),
})
export type RelatedRowsSource = z.infer<typeof RelatedRowsSource>

/**
 * A value the console binds, ready for its Parameters panel.
 *
 * `type` is the JSON name of the Rust field `kind`, a plain field and not a
 * tag: `BoundValue` is a struct, not a union.
 */
export const BoundValue = z.object({
  type: ParameterKind,
  text: z.string(),
})
export type BoundValue = z.infer<typeof BoundValue>

export const RelatedRowsQuery = z.object({
  sql: z.string(),
  parameters: z.array(BoundValue),
  /** A parameter is left for the user to fill before running. */
  needsValues: z.boolean(),
})
export type RelatedRowsQuery = z.infer<typeof RelatedRowsQuery>

export const CatalogSearchHit = z.object({
  address: CatalogAddress,
  name: z.string(),
  kind: z.string(),
  holdsRecords: z.boolean(),
  /** What made it match. Unknown to `other`, as `source` above (front.md). */
  matched: z
    .enum(["relationName", "fieldName", "comment", "other"])
    .catch("other"),
  matchedFields: z.array(z.string()),
})
export type CatalogSearchHit = z.infer<typeof CatalogSearchHit>

export const RefreshSignal = z.discriminatedUnion("type", [
  z.object({ type: z.literal("catalogInvalidated"), connection: z.string() }),
  z.object({ type: z.literal("rowsChanged"), connection: z.string() }),
  z.object({ type: z.literal("lagged") }),
])
export type RefreshSignal = z.infer<typeof RefreshSignal>

export const metadata = {
  previewRelation: (
    commandId: string,
    connection: string,
    session: string,
    address: CatalogAddress,
    shape: PreviewShape
  ) =>
    call("preview_relation", CommandOutcome, {
      commandId,
      connection,
      session,
      address,
      shape,
    }),

  previewPagination: (
    connection: string,
    address: CatalogAddress,
    shape: PreviewShape,
    rows: number
  ) =>
    call("preview_pagination", Pagination, {
      connection,
      address,
      shape,
      rows,
    }),

  refreshCatalog: (
    commandId: string,
    connection: string,
    session: string,
    address: CatalogAddress | null
  ) =>
    call("refresh_catalog", CommandOutcome, {
      commandId,
      connection,
      session,
      address,
    }),

  refreshRelationFacet: (
    commandId: string,
    connection: string,
    session: string,
    address: CatalogAddress,
    facet: RelationFacet
  ) =>
    call("refresh_relation_facet", CommandOutcome, {
      commandId,
      connection,
      session,
      address,
      facet,
    }),

  catalogTree: (connection: string) =>
    call("catalog_tree", z.array(CatalogNode), { connection }),

  relationFacets: (connection: string, address: CatalogAddress) =>
    call("relation_facets", RelationFacets, { connection, address }),

  searchCatalog: (connection: string, query: string) =>
    call("search_catalog", z.array(CatalogSearchHit), { connection, query }),

  /**
   * The query a console opens to see the rows on the other side of a key.
   *
   * With `source`, the backend reads the key's values **from the result
   * buffer** — the front holds formatted cells only — and returns them as
   * values to bind. Without it, the template keeps an empty parameter.
   */
  relatedRowsTemplate: (
    connection: string,
    address: CatalogAddress,
    incoming: boolean,
    index: number,
    source?: RelatedRowsSource | null
  ) =>
    call("related_rows_template", RelatedRowsQuery.nullable(), {
      connection,
      address,
      incoming,
      index,
      source: source ?? null,
    }),

  subscribeRefreshSignals: (onSignal: (signal: RefreshSignal) => void) => {
    // Constructing a channel outside the webview throws before `call` can
    // explain why.
    if (!isTauri()) return call("subscribe_refresh_signals", Nothing)
    const channel = new Channel<unknown>()
    channel.onmessage = guarded(
      "subscribe_refresh_signals",
      RefreshSignal,
      onSignal
    )
    return call("subscribe_refresh_signals", Nothing, { channel })
  },
}
