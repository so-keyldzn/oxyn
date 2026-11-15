import { z } from "zod"
import { call } from "./client"

export const MetadataLevel = z.enum(["minimal", "structure", "extended"])
export type MetadataLevel = z.infer<typeof MetadataLevel>
export const MetadataScope = z
  .object({ catalog: z.string().nullable(), namespace: z.string().nullable() })
  .strict()
export type MetadataScope = z.infer<typeof MetadataScope>
export const MetadataLoadingPolicy = z
  .object({
    version: z.literal(1),
    default: MetadataLevel,
    overrides: z.array(
      z.object({ scope: MetadataScope, level: MetadataLevel }).strict()
    ),
  })
  .strict()
export type MetadataLoadingPolicy = z.infer<typeof MetadataLoadingPolicy>
export const MetadataPolicySnapshot = z
  .object({
    revision: z.number().int().nonnegative().max(Number.MAX_SAFE_INTEGER),
    policy: MetadataLoadingPolicy,
  })
  .strict()
export type MetadataPolicySnapshot = z.infer<typeof MetadataPolicySnapshot>

export const MetadataPreloadCursor = z
  .object({
    generation: z.string().uuid(),
    scope: MetadataScope,
    policyRevision: z.number().int().nonnegative(),
    after: z.string(),
  })
  .strict()
export type MetadataPreloadCursor = z.infer<typeof MetadataPreloadCursor>
export const MetadataPreloadRequest = z
  .object({
    connection: z.string(),
    session: z.string(),
    scope: MetadataScope,
    expectedRevision: z.number().int().nonnegative(),
    refresh: z.boolean(),
    after: MetadataPreloadCursor.nullable(),
  })
  .strict()
export type MetadataPreloadRequest = z.infer<typeof MetadataPreloadRequest>
export const MetadataFacet = z.enum([
  "names",
  "description",
  "indexes",
  "outgoingKeys",
  "definition",
])
export const MetadataPreloadProgress = z.object({
  command: z.string(),
  connection: z.string(),
  session: z.string(),
  scope: MetadataScope,
  level: MetadataLevel,
  policyRevision: z.number().int().nonnegative(),
  state: z.enum([
    "running",
    "cancelling",
    "complete",
    "partial",
    "cancelled",
    "failed",
  ]),
  planned: z.number().int().nonnegative(),
  reads: z.number().int().nonnegative(),
  loaded: z.number().int().nonnegative(),
  cached: z.number().int().nonnegative(),
  failed: z.number().int().nonnegative(),
  unavailable: z.number().int().nonnegative(),
  samplingRequired: z.number().int().nonnegative(),
  knownRelations: z.number().int().nonnegative().nullable(),
  unreadRelations: z.number().int().nonnegative(),
  availableFacets: z.number().int().nonnegative().nullable(),
  staleFacets: z.number().int().nonnegative().nullable(),
  unreadFacets: z.number().int().nonnegative().nullable(),
  metadataBytes: z.number().nonnegative(),
  elapsedMs: z.number().nonnegative(),
  stopped: z.string().nullable(),
  next: MetadataPreloadCursor.nullable(),
  failures: z.array(
    z.object({ relation: z.string().nullable(), facet: MetadataFacet })
  ),
  omittedFailures: z.number().int().nonnegative(),
})
export type MetadataPreloadProgress = z.infer<typeof MetadataPreloadProgress>

export const metadataLoading = {
  preload: (commandId: string, request: MetadataPreloadRequest) =>
    call("preload_metadata", MetadataPreloadProgress, { commandId, request }),
  progress: (commandId: string) =>
    call("metadata_preload_progress", MetadataPreloadProgress.nullable(), {
      commandId,
    }),
  read: (commandId: string, connection: string) =>
    call("read_metadata_policy", MetadataPolicySnapshot, {
      commandId,
      connection,
    }),
  save: (
    commandId: string,
    connection: string,
    policy: MetadataLoadingPolicy,
    expectedRevision: number
  ) =>
    call("write_metadata_policy", MetadataPolicySnapshot, {
      commandId,
      connection,
      policy,
      expectedRevision,
    }),
}
