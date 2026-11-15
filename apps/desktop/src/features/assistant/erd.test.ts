import { describe, expect, it, vi } from "vitest"
import { assembleErd, resolveName } from "./erd"
import { mapFixture } from "@/components/oxyn/schema-map-fixtures"
import type { CatalogSearchHit } from "@/lib/ipc/metadata"
import type { CatalogAddress } from "@/lib/ipc/types"

const at = (namespace: string, relation: string): CatalogAddress => ({
  catalog: null,
  namespace,
  relation,
})
const hit = (address: CatalogAddress): CatalogSearchHit => ({
  address,
  name: address.relation ?? "",
  kind: "table",
  holdsRecords: true,
  matched: "relationName",
  matchedFields: [],
})
describe("resolving a name the answer wrote", () => {
  const orders = at("public", "orders")
  const archived = at("archive", "orders")

  it("takes the exact spelling, then a unique match ignoring case", () => {
    expect(
      resolveName({ namespace: null, relation: "Orders", written: "Orders" }, [
        hit(orders),
      ])
    ).toEqual({ kind: "found", address: orders })
  })

  it("never settles two candidates by a guess", () => {
    const resolution = resolveName(
      { namespace: null, relation: "orders", written: "orders" },
      [hit(orders), hit(archived)]
    )
    expect(resolution.kind).toBe("ambiguous")
    expect(
      resolveName(
        { namespace: "archive", relation: "orders", written: "archive.orders" },
        [hit(orders), hit(archived)]
      )
    ).toEqual({ kind: "found", address: archived })
  })

  it("does not take a column match for a table", () => {
    expect(
      resolveName({ namespace: null, relation: "ghost", written: "ghost" }, [
        hit(orders),
      ])
    ).toEqual({ kind: "notFound" })
  })
})

describe("shared host projection", () => {
  it("resolves names once, preserves host identities and never guesses ambiguous homonyms", async () => {
    const project = vi.fn(async () => mapFixture())
    const search = vi.fn(async () => [
      hit(at("sales", "orders")),
      hit(at("archive", "orders")),
    ])
    const result = await assembleErd(
      [
        { namespace: null, relation: "orders", written: "orders" },
        { namespace: "sales", relation: "orders", written: "sales.orders" },
      ],
      2,
      { project, search }
    )
    expect(search).toHaveBeenCalledTimes(1)
    expect(project).toHaveBeenCalledExactlyOnceWith([at("sales", "orders")])
    expect(result.ambiguous).toHaveLength(1)
    expect(result.tables[0]?.key).toBe(mapFixture().projection.objects[0]?.key)
    expect(result.links[0]?.fields).toEqual(["tenant", "parent_id"])
    expect(result.ignoredNames).toBe(2)
    expect(result.coverageNotice).toContain(
      "Incoming links cover loaded objects only"
    )
  })
  it("does not call the native loader when none of the names resolve", async () => {
    const project = vi.fn(async () => mapFixture())
    const result = await assembleErd(
      [{ namespace: null, relation: "absent", written: "absent" }],
      0,
      { search: async () => [], project }
    )
    expect(project).not.toHaveBeenCalled()
    expect(result.tables).toEqual([])
    expect(result.notFound).toEqual(["absent"])
  })
  it("preserves failures rather than presenting a successful empty map", async () => {
    await expect(
      assembleErd(
        [{ namespace: "sales", relation: "orders", written: "sales.orders" }],
        0,
        {
          search: async () => [hit(at("sales", "orders"))],
          project: async () => {
            throw new Error("source closed")
          },
        }
      )
    ).rejects.toThrow("source closed")
  })
})
