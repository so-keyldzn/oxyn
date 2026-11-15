import { z } from "zod"
import { call } from "./client"
import { CatalogAddress } from "./types"
import { MetadataScope } from "./metadata-loading"
import { metadata, SchemaSampleLimits } from "./metadata"

export const MapObjectRef = z
  .object({ scope: MetadataScope, relation: z.string() })
  .strict()
export type MapObjectRef = z.infer<typeof MapObjectRef>
export const SchemaMapRequest = z
  .object({
    connection: z.string(),
    session: z.string(),
    scope: MetadataScope,
    objects: z.array(MapObjectRef),
    expand: MapObjectRef.nullable(),
    refresh: z.boolean(),
  })
  .strict()
export type SchemaMapRequest = z.infer<typeof SchemaMapRequest>
export const MapFreshness = z.enum(["unread", "fresh", "stale"])
export const MapField = z.object({
  key: z.string(),
  parent: z.string().nullable(),
  path: z.array(z.string()),
  name: z.string(),
  rawType: z.string(),
  typeShortened: z.boolean(),
  nullable: z.boolean(),
  primaryKey: z.boolean(),
  origin: z.enum(["reported", "sampled", "previewLayout"]),
})
export type MapField = z.infer<typeof MapField>
export const MapObject = z.object({
  key: z.string(),
  address: CatalogAddress,
  kind: z.string(),
  holdsRecords: z.boolean(),
  fields: z.array(MapField),
  omittedFields: z.number().int().nonnegative(),
  fieldsState: MapFreshness,
  samplingRequired: z.boolean(),
  keysState: MapFreshness,
  keysSupported: z.boolean(),
})
export type MapObject = z.infer<typeof MapObject>
export const MapLink = z.object({
  key: z.string(),
  kind: z.enum(["foreignKey", "dependency", "membership", "containment"]),
  from: z.string(),
  to: z.string(),
  target: CatalogAddress,
  targetLoaded: z.boolean(),
  name: z.string(),
  fields: z.array(z.string()),
  referencedFields: z.array(z.string()),
  wellFormed: z.boolean(),
  freshness: MapFreshness,
  sourceUnique: z.boolean().nullable(),
  targetUnique: z.boolean().nullable(),
})
export type MapLink = z.infer<typeof MapLink>
export const SchemaMapProjection = z.object({
  source: z.object({
    connection: z.string(),
    session: z.string(),
    generation: z.string().uuid(),
  }),
  containers: z.array(
    z.object({ key: z.string(), address: CatalogAddress, level: z.string() })
  ),
  objects: z.array(MapObject),
  links: z.array(MapLink),
  coverage: z.object({
    knownObjects: z.number().int().nonnegative(),
    drawnObjects: z.number().int().nonnegative(),
    omittedObjects: z.number().int().nonnegative(),
    omittedFields: z.number().int().nonnegative(),
    omittedLinks: z.number().int().nonnegative(),
    unreadObjects: z.number().int().nonnegative(),
    staleObjects: z.number().int().nonnegative(),
    unresolvedLinks: z.number().int().nonnegative(),
  }),
  dependenciesSupported: z.boolean(),
  sampleLimits: SchemaSampleLimits.nullable(),
})
export type SchemaMapProjection = z.infer<typeof SchemaMapProjection>
export const SchemaMapLoaded = z.object({
  projection: SchemaMapProjection,
  issues: z.array(
    z.object({
      address: CatalogAddress,
      facet: z.enum(["names", "fields", "outgoingKeys", "incomingKeys"]),
      kind: z.enum(["failed", "unavailable", "notObserved", "omitted"]),
    })
  ),
  reads: z.number().int().nonnegative(),
  chooseNamespace: z.boolean(),
  stopped: z.string().nullable(),
})
export type SchemaMapLoaded = z.infer<typeof SchemaMapLoaded>
export const schemaMaps = {
  load: async (commandId: string, request: SchemaMapRequest) => {
    const map = await call("load_schema_map", SchemaMapLoaded, {
      commandId,
      request,
    })
    if (
      map.projection.source.connection !== request.connection ||
      map.projection.source.session !== request.session
    )
      throw new Error("The map answered for another owner.")
    // A native read can be current when published yet superseded while its
    // IPC response is delivered. This second read is memory-only, never I/O.
    const generation = await metadata.catalogGeneration(
      request.connection,
      request.session
    )
    if (generation !== map.projection.source.generation)
      throw new Error(
        "The source changed before the map arrived. Refresh its metadata."
      )
    return map
  },
}
