import { describe, expect, it } from "vitest"

import {
  addressKey,
  boundedFocus,
  typeaheadMatch,
  visibleRows,
} from "./catalog-tree"
import type { CatalogNode } from "@/lib/ipc/types"

const relation = (name: string): CatalogNode => ({
  address: { catalog: null, namespace: "public", relation: name },
  name,
  kind: "table",
  holdsRecords: true,
  system: false,
  comment: null,
  loaded: true,
  stale: false,
  children: [],
})

const schema: CatalogNode = {
  address: { catalog: null, namespace: "public", relation: null },
  name: "public",
  kind: "namespace",
  holdsRecords: false,
  system: false,
  comment: null,
  loaded: true,
  stale: false,
  children: [relation("orders"), relation("users")],
}

describe("catalog tree", () => {
  it("shows children only once their parent is expanded", () => {
    expect(visibleRows([schema], new Set(), "")).toHaveLength(1)
    const open = new Set([addressKey(schema.address)])
    expect(visibleRows([schema], open, "").map((row) => row.node.name)).toEqual(
      ["public", "orders", "users"]
    )
  })

  it("filters loaded objects and keeps their ancestors", () => {
    const rows = visibleRows([schema], new Set(), "ORD")
    expect(rows.map((row) => row.node.name)).toEqual(["public", "orders"])
  })

  it("keeps a hostile dotted name as one segment", () => {
    const hostile = relation('users"; DROP TABLE audit; --.x')
    const other = {
      ...relation("x"),
      address: {
        catalog: null,
        namespace: 'public.users"; DROP TABLE audit; --',
        relation: "x",
      },
    }
    expect(addressKey(hostile.address)).not.toEqual(addressKey(other.address))
  })

  it("gives each row its place among its visible siblings and its parent", () => {
    const open = new Set([addressKey(schema.address)])
    const rows = visibleRows([schema], open, "")
    expect(rows.map((row) => [row.posInSet, row.setSize])).toEqual([
      [1, 1],
      [1, 2],
      [2, 2],
    ])
    expect(rows[2]?.parentKey).toBe(addressKey(schema.address))
    expect(rows[0]?.parentKey).toBeNull()
    // Under a filter, the set is what is shown, not what is loaded.
    const filtered = visibleRows([schema], new Set(), "users")
    expect(filtered[1]?.setSize).toBe(1)
  })

  it("keeps focus on its row, and bounds it when the row disappears", () => {
    const rows = [{ key: "a" }, { key: "b" }, { key: "c" }]
    expect(boundedFocus(rows, "b", 0)).toBe(1)
    expect(boundedFocus(rows, "gone", 7)).toBe(2)
    expect(boundedFocus(rows, null, 1)).toBe(1)
    expect(boundedFocus([], "a", 3)).toBe(-1)
  })

  it("moves by typed prefix and cycles on a repeated letter", () => {
    const rows = ["orders", "outbox", "users", "old_orders"]
      .map((name) => relation(name))
      .map((node) => ({ node }))
    expect(typeaheadMatch(rows, "o", 0)).toBe(1)
    expect(typeaheadMatch(rows, "o", 1)).toBe(3)
    expect(typeaheadMatch(rows, "o", 3)).toBe(0)
    expect(typeaheadMatch(rows, "us", 0)).toBe(2)
    expect(typeaheadMatch(rows, "ol", 2)).toBe(3)
    expect(typeaheadMatch(rows, "zz", 0)).toBe(-1)
  })
})
