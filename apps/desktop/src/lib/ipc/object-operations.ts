// Mirror of `crates/oxyn-desktop/src/ipc/object_operations.rs`, as executable
// schemas (ADR-0031). The front sends an operation and shows the statement the
// backend composed; it never composes one (ADR-0042, I-10).

import { z } from "zod"

import { call } from "./client"
import { Facet, IncomingKeyRow } from "./metadata"
import { ApprovalPreview, Environment } from "./types"
import type { CatalogAddress } from "./types"

/** What the review composes, as the user chose it in the box. */
export type ObjectOperation =
  | { kind: "drop"; cascade: boolean }
  | { kind: "truncate"; cascade: boolean }
  | { kind: "rename"; column: string | null; newName: string }

/** What the submitted statement must be: what the button said. */
export type OperationKind = ObjectOperation["kind"]

export const Dependents = z.discriminatedUnion("type", [
  z.object({ type: z.literal("notApplicable") }),
  z.object({ type: z.literal("notReported") }),
  // A newtype variant: serde flattens the facet beside its tag (front.md).
  Facet(z.array(IncomingKeyRow)).extend({
    type: z.literal("incomingKeys"),
  }),
])
export type Dependents = z.infer<typeof Dependents>

export const ObjectOperationReview = z.object({
  /** The one statement that will be submitted, exactly. */
  sql: z.string(),
  connectionName: z.string(),
  environment: Environment,
  /** The unqualified name typed on production. */
  objectName: z.string(),
  relationKind: z.string(),
  transactionalDdl: z.boolean(),
  restrictDependents: z.boolean(),
  dependents: Dependents,
})
export type ObjectOperationReview = z.infer<typeof ObjectOperationReview>

export const ObjectOperationOutcome = z.discriminatedUnion("type", [
  z.object({ type: z.literal("applied") }),
  z.object({
    type: z.literal("needsApproval"),
    command: z.string(),
    reason: z.string(),
    preview: ApprovalPreview.nullable(),
  }),
  z.object({ type: z.literal("denied"), reason: z.string() }),
  z.object({ type: z.literal("failed"), message: z.string() }),
  z.object({ type: z.literal("ambiguous"), message: z.string() }),
  z.object({ type: z.literal("notSent"), message: z.string() }),
])
export type ObjectOperationOutcome = z.infer<typeof ObjectOperationOutcome>

export const objectOperations = {
  review: (
    connection: string,
    session: string,
    address: CatalogAddress,
    operation: ObjectOperation
  ) =>
    call("review_object_operation", ObjectOperationReview, {
      connection,
      session,
      address,
      operation,
    }),

  run: (
    commandId: string,
    connection: string,
    operation: OperationKind,
    sql: string
  ) =>
    call("run_object_operation", ObjectOperationOutcome, {
      commandId,
      connection,
      operation,
      sql,
    }),
}
