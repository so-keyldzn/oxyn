import { z } from "zod"
import { call } from "./client"

export const NativeOperation = z.enum(["export", "restore"])
export type NativeOperation = z.infer<typeof NativeOperation>
export const NativeFormat = z.enum(["customArchive", "sql"])
export type NativeFormat = z.infer<typeof NativeFormat>
export const NativeToolRole = z.enum(["export", "restore"])
export const NativeToolIdentity = z.object({
  engine: z.string(),
  role: NativeToolRole,
  name: z.string(),
  version: z.string(),
})
export type NativeToolIdentity = z.infer<typeof NativeToolIdentity>
export const NativeTool = z.object({
  grant: z.string().uuid(),
  identity: NativeToolIdentity,
})
export type NativeTool = z.infer<typeof NativeTool>
export const NativeFile = z.object({
  grant: z.string().uuid(),
  name: z.string(),
})
export type NativeFile = z.infer<typeof NativeFile>
export const NativeSupport = z.object({
  engine: z.string(),
  export: z.boolean(),
  restore: z.boolean(),
  reason: z.string().nullable(),
  exportFormats: z.array(NativeFormat),
  restoreFormats: z.array(NativeFormat),
})
export type NativeSupport = z.infer<typeof NativeSupport>
export const NativeJob = z.object({
  version: z.literal(1),
  id: z.string().uuid(),
  connection: z.string(),
  session: z.string(),
  owner: z.string(),
  command: z.string().nullable(),
  connectionName: z.string(),
  database: z.string(),
  engine: z.string(),
  operation: NativeOperation,
  format: NativeFormat,
  noOwner: z.boolean(),
  noPrivileges: z.boolean(),
  tool: NativeToolIdentity,
  fileName: z.string(),
  stage: z.enum([
    "preparing",
    "ready",
    "running",
    "finalizing",
    "cancelling",
    "finished",
  ]),
  outcome: z
    .enum([
      "succeeded",
      "refused",
      "failedBeforeStart",
      "failed",
      "cancelled",
      "possiblePartial",
      "unknown",
      "interrupted",
    ])
    .nullable(),
  failure: z
    .enum([
      "unavailable",
      "tool",
      "archive",
      "file",
      "sourceChanged",
      "process",
      "cancelled",
      "deadline",
      "cleanup",
      "interrupted",
    ])
    .nullable(),
  bytes: z.number().nonnegative(),
  mayHaveChanged: z.boolean(),
  reconciled: z.boolean(),
  createdAt: z.string(),
  finishedAt: z.string().nullable(),
})
export type NativeJob = z.infer<typeof NativeJob>
export const NativeReview = z.object({
  job: NativeJob,
  destination: z.string(),
})
export type NativeReview = z.infer<typeof NativeReview>
export const NativePreparation = z
  .object({
    connection: z.string(),
    session: z.string(),
    operation: NativeOperation,
    format: NativeFormat,
    tool: z.string(),
    file: z.string(),
    noOwner: z.boolean(),
    noPrivileges: z.boolean(),
  })
  .strict()
export type NativePreparation = z.infer<typeof NativePreparation>
export const nativeTransfers = {
  support: (commandId: string, connection: string, session: string) =>
    call("native_transfer_support", NativeSupport, {
      commandId,
      connection,
      session,
    }),
  selectTool: (
    commandId: string,
    connection: string,
    session: string,
    role: NativeOperation
  ) =>
    call("select_native_tool", NativeTool.nullable(), {
      commandId,
      connection,
      session,
      role,
    }),
  selectFile: (
    connection: string,
    session: string,
    operation: NativeOperation,
    format: NativeFormat
  ) =>
    call("select_native_transfer_file", NativeFile.nullable(), {
      connection,
      session,
      operation,
      format,
    }),
  prepare: (commandId: string, request: NativePreparation) =>
    call("prepare_native_transfer", NativeReview, { commandId, request }),
  start: (
    commandId: string,
    connection: string,
    session: string,
    transfer: string
  ) =>
    call("start_native_transfer", NativeJob, {
      commandId,
      connection,
      session,
      transfer,
    }),
  discard: (
    commandId: string,
    connection: string,
    session: string,
    transfer: string
  ) =>
    call("discard_native_transfer", NativeJob, {
      commandId,
      connection,
      session,
      transfer,
    }),
  jobs: (commandId: string, connection: string, session: string) =>
    call("native_transfer_jobs", z.array(NativeJob), {
      commandId,
      connection,
      session,
    }),
  reconcile: (
    commandId: string,
    connection: string,
    session: string,
    transfer: string
  ) =>
    call("reconcile_native_transfer", NativeJob, {
      commandId,
      connection,
      session,
      transfer,
    }),
}

export const nativeSelections = {
  revoke: (commandId: string, grant: string) =>
    call("revoke_native_selection", z.null(), { commandId, grant }),
  progress: (connection: string, transfer: string) =>
    call("native_transfer_progress", NativeJob.nullable(), {
      connection,
      transfer,
    }),
}
