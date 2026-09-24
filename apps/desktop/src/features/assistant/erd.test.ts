import { describe, expect, it, vi } from "vitest"

import { assembleErd, resolveName } from "./erd"
import type { ErdSources } from "./erd"
import { ERD_MAX_TABLES } from "@/components/oxyn/erd-diagram"
import type {
  CatalogSearchHit,
  ForeignKeyRow,
  RelationFacets,
} from "@/lib/ipc/metadata"
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

const key = (
  name: string,
  fields: Array<string>,
  references: CatalogAddress
): ForeignKeyRow => ({
  name,
  fields,
  references,
  referencedFields: ["id"],
  onDelete: "NO ACTION",
  cascades: false,
  wellFormed: true,
})

const fetched = { state: "fetched" as const, fetchedAt: "2026-09-24T10:00:00Z" }

function facets(
  address: CatalogAddress,
  foreignKeys: Array<ForeignKeyRow>,
  incoming: Array<CatalogAddress> = []
): RelationFacets {
  return {
    address,
    kind: "table",
    holdsRecords: true,
    qualifiedName: `"${address.namespace}"."${address.relation}"`,
    detail: {
      freshness: fetched,
      value: {
        name: address.relation ?? "",
        kind: "table",
        comment: null,
        estimatedRows: null,
        sizeBytes: null,
        fields: [
          {
            name: "id",
            position: 1,
            logicalType: "integer",
            rawType: "int4",
            nullable: false,
            default: null,
            comment: null,
            primaryKey: true,
          },
          ...foreignKeys.flatMap((row, index) =>
            row.fields.map((field) => ({
              name: field,
              position: index + 2,
              logicalType: "integer",
              rawType: "int4",
              nullable: true,
              default: null,
              comment: null,
              primaryKey: false,
            }))
          ),
        ],
      },
    },
    indexes: [],
    foreignKeys,
    constraints: { freshness: { state: "never" }, value: null },
    incomingKeys: {
      freshness: fetched,
      value: incoming.map((source) => ({
        source,
        key: key("in", ["x"], address),
        sourceUnique: false,
      })),
    },
    definition: { freshness: { state: "never" }, value: null },
    uniqueKey: true,
  }
}

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

describe("assembling the diagram", () => {
  const customers = at("public", "customers")
  const orders = at("public", "orders")
  const items = at("public", "order_items")
  const products = at("public", "products")

  const catalog = new Map<string, RelationFacets>([
    ["customers", facets(customers, [], [orders])],
    [
      "orders",
      facets(
        orders,
        [key("orders_customer", ["customer_id"], customers)],
        [items]
      ),
    ],
    [
      "order_items",
      facets(items, [
        key("items_order", ["order_id"], orders),
        key("items_product", ["product_id"], products),
      ]),
    ],
    ["products", facets(products, [], [items])],
  ])

  const sources = (): ErdSources => ({
    search: vi.fn(async (relation: string) => {
      const found = catalog.get(relation)
      return found ? [hit(found.address)] : []
    }),
    facets: vi.fn(async (address: CatalogAddress) => {
      const found = catalog.get(address.relation ?? "")
      if (!found)
        throw new Error(`relation "${address.relation}" does not exist`)
      return found
    }),
  })

  it("draws the named tables, their direct neighbours and the keys between them", async () => {
    const erd = await assembleErd(
      [
        { namespace: null, relation: "orders", written: "orders" },
        { namespace: null, relation: "ghost", written: "ghost" },
      ],
      0,
      sources()
    )
    expect(erd.notFound).toEqual(["ghost"])
    expect(erd.tables.map((table) => [table.name, table.requested])).toEqual([
      ["orders", true],
      ["customers", false],
      ["order_items", false],
    ])
    // products is two keys away: not drawn, and its key not either.
    expect(erd.links.map((link) => link.name).sort()).toEqual([
      "items_order",
      "orders_customer",
    ])
    const ordersTable = erd.tables[0]
    expect(
      ordersTable?.columns.find((c) => c.name === "customer_id")
    ).toMatchObject({
      foreignKey: true,
      primaryKey: false,
    })
    expect(ordersTable?.columns.find((c) => c.name === "id")?.primaryKey).toBe(
      true
    )
  })

  it("stops at the bound and counts what it left out", async () => {
    const hub = at("public", "hub")
    const spokes = Array.from({ length: ERD_MAX_TABLES + 5 }, (_, i) =>
      at("public", `spoke_${i}`)
    )
    const erd = await assembleErd(
      [{ namespace: null, relation: "hub", written: "hub" }],
      2,
      {
        search: async () => [hit(hub)],
        facets: async (address) =>
          address.relation === "hub"
            ? facets(hub, [], spokes)
            : facets(address, [key("to_hub", ["hub_id"], hub)]),
      }
    )
    expect(erd.tables).toHaveLength(ERD_MAX_TABLES)
    expect(erd.omitted).toBe(6)
    expect(erd.ignoredNames).toBe(2)
    expect(erd.links).toHaveLength(ERD_MAX_TABLES - 1)
  })

  it("lets a failed read fail the diagram with the server's words", async () => {
    await expect(
      assembleErd(
        [{ namespace: null, relation: "orders", written: "orders" }],
        0,
        {
          search: async () => [hit(orders)],
          facets: async () => {
            throw new Error("permission denied for table orders")
          },
        }
      )
    ).rejects.toThrow("permission denied for table orders")
  })
})
