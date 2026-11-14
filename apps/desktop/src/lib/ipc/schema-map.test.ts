import { beforeEach, expect, it, vi } from "vitest"
import { mapFixture } from "@/components/oxyn/schema-map-fixtures"
import { schemaMaps } from "./schema-map"
import type { SchemaMapRequest } from "./schema-map"
import type * as MetadataModule from "./metadata"

const rpc = vi.hoisted(() => ({ call: vi.fn(), generation: vi.fn() }))
vi.mock("./client", () => ({ call: rpc.call }))
vi.mock("./metadata", async (original) => ({
  ...(await original<typeof MetadataModule>()),
  metadata: { catalogGeneration: rpc.generation },
}))
const request: SchemaMapRequest = {
  connection: "fixture-source",
  session: "fixture-owner",
  scope: { catalog: null, namespace: null },
  objects: [],
  expand: null,
  refresh: false,
}
beforeEach(() => {
  rpc.call.mockReset()
  rpc.generation.mockReset()
})
it("rejects a response superseded while crossing IPC, on every map consumer", async () => {
  rpc.call.mockResolvedValue(mapFixture())
  rpc.generation.mockResolvedValue("00000000-0000-4000-8000-000000000002")
  await expect(schemaMaps.load("request", request)).rejects.toThrow(
    "source changed"
  )
  expect(rpc.generation).toHaveBeenCalledExactlyOnceWith(
    request.connection,
    request.session
  )
})
it("does not consult another owner's cache or accept a replaced session", async () => {
  const map = mapFixture()
  map.projection.source.session = "another-owner"
  rpc.call.mockResolvedValue(map)
  await expect(schemaMaps.load("request", request)).rejects.toThrow(
    "another owner"
  )
  expect(rpc.generation).not.toHaveBeenCalled()
})
it("returns the same projection when its exact memory epoch still exists", async () => {
  const map = mapFixture()
  rpc.call.mockResolvedValue(map)
  rpc.generation.mockResolvedValue(map.projection.source.generation)
  await expect(schemaMaps.load("request", request)).resolves.toBe(map)
  expect(rpc.call).toHaveBeenCalledTimes(1)
})
