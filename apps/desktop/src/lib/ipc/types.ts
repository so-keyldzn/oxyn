// Mirror of `crates/oxyn-desktop/src/ipc.rs`, as executable schemas: a field
// renamed on one side without the other is now caught at the boundary, by name,
// on the first call — instead of surfacing as an `undefined` three screens away
// (ADR-0031). The two sides still change in the same commit; the difference is
// that forgetting is no longer silent.
//
// Types are derived with `z.infer`, so a schema and its type cannot drift.

import { z } from "zod"

export const Environment = z.enum([
  "local",
  "development",
  "staging",
  "production",
])
export type Environment = z.infer<typeof Environment>

export const ENVIRONMENTS: ReadonlyArray<Environment> = [
  "production",
  "staging",
  "development",
  "local",
]

/** What may leave the machine when an agent speaks (ADR-0006, I-04). */
export const PrivacyTier = z.enum(["local", "metadata", "sampled"])
export type PrivacyTier = z.infer<typeof PrivacyTier>

export const IpcError = z.object({
  message: z.string(),
  retryable: z.boolean(),
})
export type IpcError = z.infer<typeof IpcError>

export const FormFieldKind = z.discriminatedUnion("type", [
  z.object({ type: z.literal("text") }),
  z.object({ type: z.literal("password") }),
  z.object({ type: z.literal("number") }),
  z.object({ type: z.literal("bool") }),
  z.object({ type: z.literal("choice"), options: z.array(z.string()) }),
  z.object({ type: z.literal("path") }),
])
export type FormFieldKind = z.infer<typeof FormFieldKind>

export const FormField = z.object({
  key: z.string(),
  label: z.string(),
  kind: FormFieldKind,
  required: z.boolean(),
  secret: z.boolean(),
  default: z.string().nullable(),
  help: z.string().nullable(),
})
export type FormField = z.infer<typeof FormField>

export const DriverChoice = z.object({
  id: z.string(),
  displayName: z.string(),
  family: z.string(),
  defaultPort: z.number().int().nullable(),
  fields: z.array(FormField),
})
export type DriverChoice = z.infer<typeof DriverChoice>

export const SavedConnection = z.object({
  id: z.string(),
  name: z.string(),
  driver: z.string(),
  environment: Environment,
  readOnly: z.boolean(),
  privacyTier: PrivacyTier,
})
export type SavedConnection = z.infer<typeof SavedConnection>

export const ConnectionDraft = z.object({
  driver: z.string(),
  name: z.string(),
  /**
   * Required by `ipc.rs`, which refuses to default it: a missing marking is a
   * front bug, and choosing one here would hide it (I-02).
   */
  environment: Environment,
  /** Required for the same reason, and for the same kind of stake (I-04). */
  privacyTier: PrivacyTier,
  readOnly: z.boolean(),
  values: z.record(z.string(), z.string()),
  secrets: z.record(z.string(), z.string()),
})
export type ConnectionDraft = z.infer<typeof ConnectionDraft>

/**
 * Whether a session has a transaction open, as the session reported it
 * (ADR-0039). `unknown` is never shown as `idle`.
 */
export const TransactionState = z.enum(["idle", "open", "unknown"])
export type TransactionState = z.infer<typeof TransactionState>

/**
 * A console's own session.
 *
 * Declared here rather than next to the console commands because
 * `OpenConnection` carries one: with schemas being values, a module cycle is no
 * longer erased at compile time the way `import type` was — it would evaluate
 * to `undefined` at load. `lib/ipc/consoles.ts` re-exports it.
 */
export const ConsoleSession = z.object({
  session: z.string(),
  capabilities: z.array(z.string()),
  readOnly: z.boolean(),
  /** What the session reported at opening (ADR-0039 §4). */
  transactionState: TransactionState,
})
export type ConsoleSession = z.infer<typeof ConsoleSession>

export const OpenConnection = z.object({
  connection: z.string(),
  session: z.string(),
  name: z.string(),
  driver: z.string(),
  environment: Environment,
  readOnly: z.boolean(),
  privacyTier: PrivacyTier,
  capabilities: z.array(z.string()),
  /** The first console's own session; `session` stays the catalog's (ADR-0015). */
  console: ConsoleSession,
})
export type OpenConnection = z.infer<typeof OpenConnection>

/**
 * `estimatedRows` carries no `.int()`, unlike the counters Oxyn bounds itself.
 *
 * It is a `u64` a server reports, and zod's `.int()` is `Number.isSafeInteger`:
 * past 2^53 it would reject an honest answer. The same reasoning applies to
 * every `u64` that comes from the other side rather than from a buffer we size.
 */
export const ApprovalPreview = z.object({
  statement: z.string(),
  connection: z.string(),
  estimatedRows: z.number().nonnegative().nullable(),
})
export type ApprovalPreview = z.infer<typeof ApprovalPreview>

export const ConnectResponse = z.discriminatedUnion("type", [
  OpenConnection.extend({ type: z.literal("open") }),
  z.object({
    type: z.literal("approval"),
    command: z.string(),
    reason: z.string(),
    preview: ApprovalPreview.nullable(),
  }),
])
export type ConnectResponse = z.infer<typeof ConnectResponse>

/**
 * The answer to `test_connection`. A failed test is an answer, not a rejected
 * call: `class` is `ErrorClass::as_str`, an open set on the Rust side, so it
 * stays a string; `retryable` comes from it, never from the message (I-13).
 */
export const ConnectionTest = z.discriminatedUnion("type", [
  z.object({
    type: z.literal("succeeded"),
    elapsedMs: z.number().int().nonnegative(),
  }),
  z.object({
    type: z.literal("failed"),
    message: z.string(),
    class: z.string(),
    retryable: z.boolean(),
  }),
  z.object({ type: z.literal("cancelled") }),
])
export type ConnectionTest = z.infer<typeof ConnectionTest>

export const ResultColumn = z.object({
  name: z.string(),
  dataType: z.string(),
  nullable: z.boolean(),
})
export type ResultColumn = z.infer<typeof ResultColumn>

export const CommandOutcome = z.discriminatedUnion("type", [
  z.object({
    type: z.literal("executed"),
    result: z.string(),
    columns: z.array(ResultColumn),
    rows: z.number().int().nonnegative(),
    elapsedMs: z.number().nonnegative(),
    complete: z.boolean(),
    cancelled: z.boolean(),
    truncated: z.boolean(),
  }),
  z.object({
    type: z.literal("needsApproval"),
    command: z.string(),
    reason: z.string(),
    preview: ApprovalPreview.nullable(),
  }),
  z.object({ type: z.literal("denied"), reason: z.string() }),
  z.object({ type: z.literal("catalogRefreshed") }),
  z.object({
    type: z.literal("exported"),
    rows: z.number().int().nonnegative(),
    bytes: z.number().int().nonnegative(),
  }),
  z.object({ type: z.literal("cancelled") }),
  z.object({ type: z.literal("done") }),
])
export type CommandOutcome = z.infer<typeof CommandOutcome>

/** `null`, the text, or one of the two cases the grid draws differently. */
export type Cell =
  null | string | { text: string; fullBytes: number } | { unrenderable: string }

/**
 * Tells the four shapes apart by looking, instead of trying each in turn.
 *
 * `Cell` is the only `untagged` enum crossing the boundary, and the grid is
 * the hottest path in the product: a page holds up to `PAGE_CELLS` (20 000)
 * cells, inside an 8 ms frame budget. Measured 2026-09-16 on a full page —
 * a `z.union` of the four shapes costs 2.26 ms p50 / 7.72 ms p99, this costs
 * 0.83 ms p50 / 1.16 ms p99, for the same guarantee (ADR-0031).
 *
 * It checks the payload too, not just the key: `{ text: <object> }` would
 * otherwise reach the grid and render `NaN` for its width — the very failure
 * this boundary exists to replace with a named error. Measured at 0.85 ms p50
 * against 0.78 ms for the key-only test, on the same full page.
 */
function isCell(value: unknown): value is Cell {
  if (value === null || typeof value === "string") return true
  if (typeof value !== "object" || Array.isArray(value)) return false
  if ("text" in value) return typeof value.text === "string"
  return "unrenderable" in value && typeof value.unrenderable === "string"
}

export const Cell = z.custom<Cell>(isCell, { message: "not a cell" })

export const ResultPage = z.object({
  offset: z.number().int().nonnegative(),
  rows: z.array(z.array(Cell)),
  totalRows: z.number().int().nonnegative(),
  complete: z.boolean(),
})
export type ResultPage = z.infer<typeof ResultPage>

/**
 * A window of rows, or the answer for a result retention let go (ADR-0017).
 *
 * Tagged like the Rust enum, which means a **page** carries `type: "page"`
 * too: the two are told apart by the tag's value, never by its presence.
 * Declared once here, and not a second time next to a reader — two copies of
 * this type is how one of them came to be missing the tag.
 */
export const ResultWindow = z.discriminatedUnion("type", [
  ResultPage.extend({ type: z.literal("page") }),
  z.object({ type: z.literal("expired") }),
])
export type ResultWindow = z.infer<typeof ResultWindow>

/** Addressed by segments: a relation named `a.b` is legal (I-10). */
export const CatalogAddress = z.object({
  catalog: z.string().nullable(),
  namespace: z.string().nullable(),
  relation: z.string().nullable(),
})
export type CatalogAddress = z.infer<typeof CatalogAddress>

export interface CatalogNode {
  address: CatalogAddress
  name: string
  kind: string
  holdsRecords: boolean
  system: boolean
  comment: string | null
  loaded: boolean
  /** Read, then invalidated by a DDL sent from Oxyn (ADR-0022). */
  stale: boolean
  children: Array<CatalogNode>
}

// The tree is recursive, so the schema refers to itself: `z.infer` cannot
// unfold that on its own, and the interface above is the one it is checked
// against.
export const CatalogNode: z.ZodType<CatalogNode> = z.lazy(() =>
  z.object({
    address: CatalogAddress,
    name: z.string(),
    kind: z.string(),
    holdsRecords: z.boolean(),
    system: z.boolean(),
    comment: z.string().nullable(),
    loaded: z.boolean(),
    stale: z.boolean(),
    children: z.array(CatalogNode),
  })
)

export const RelationField = z.object({
  name: z.string(),
  position: z.number().int().nonnegative(),
  logicalType: z.string(),
  rawType: z.string(),
  nullable: z.boolean(),
  default: z.string().nullable(),
  comment: z.string().nullable(),
  primaryKey: z.boolean(),
})
export type RelationField = z.infer<typeof RelationField>

export const RelationDetail = z.object({
  name: z.string(),
  kind: z.string(),
  comment: z.string().nullable(),
  /** Both reported by the server, so neither is `.int()` — see `ApprovalPreview`. */
  estimatedRows: z.number().nonnegative().nullable(),
  sizeBytes: z.number().nonnegative().nullable(),
  fields: z.array(RelationField),
})
export type RelationDetail = z.infer<typeof RelationDetail>

/**
 * One union, where Rust has two types.
 *
 * `ExecutionEvent` carries its `ExecutionEventKind` under `#[serde(flatten)]`,
 * so `command`, the `type` tag and the variant's own fields all arrive at the
 * same level: the JSON draws no line between the two, and neither does this.
 * Splitting them back apart would mean either a second declaration of the nine
 * variants, or an intersection — which stops `type` from being a real
 * discriminant, on a union parsed once per batch of a running query.
 */
export const ExecutionEvent = z.discriminatedUnion("type", [
  z.object({
    command: z.string(),
    type: z.literal("schemaReady"),
    result: z.string(),
  }),
  z.object({
    command: z.string(),
    type: z.literal("batchReady"),
    result: z.string(),
    rows: z.number().int().nonnegative(),
  }),
  z.object({
    command: z.string(),
    type: z.literal("progress"),
    rows: z.number().int().nonnegative(),
  }),
  z.object({
    command: z.string(),
    type: z.literal("completed"),
    result: z.string(),
    rows: z.number().int().nonnegative(),
    elapsedMs: z.number().nonnegative(),
  }),
  z.object({
    command: z.string(),
    type: z.literal("failed"),
    error: z.string(),
    retryable: z.boolean(),
  }),
  z.object({
    command: z.string(),
    type: z.literal("approvalRequested"),
    reason: z.string(),
  }),
  z.object({ command: z.string(), type: z.literal("cancelled") }),
  z.object({ command: z.string(), type: z.literal("catalogUpdated") }),
  z.object({
    command: z.string(),
    type: z.literal("transactionState"),
    session: z.string(),
    state: TransactionState,
  }),
])
export type ExecutionEvent = z.infer<typeof ExecutionEvent>

// An `ExportFormat` union used to sit here, listing five names where
// `EXPORT_FORMATS` (ipc/results.rs) holds eight — `parquet`, `sql` and
// `markdown` were missing. Nothing imported it, which is how it stayed wrong:
// the front reads the list from `export_formats` and sends back the `format`
// string it was given. A mirror nobody exercises drifts unnoticed, so it is
// gone rather than merely corrected.
