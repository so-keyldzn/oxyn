import { describe, expect, it } from "vitest"

import { ERD_MAX_COLUMNS, layoutErd, visibleColumns } from "./erd-diagram"
import { shopLinks, shopTables } from "./erd-fixtures"

describe("the table diagram", () => {
  it("shows keys first and counts the columns it leaves out", () => {
    const products = shopTables.find((table) => table.name === "products")
    const { shown, hidden } = visibleColumns(products?.columns ?? [])
    expect(shown).toHaveLength(ERD_MAX_COLUMNS)
    expect(shown[0]?.primaryKey).toBe(true)
    expect(hidden).toBe(5)
  })

  it("places a referencing table after the one it references, the same way twice", () => {
    const first = layoutErd(shopTables, shopLinks)
    const orders = first.get(shopTables[1]?.key ?? "")
    const customers = first.get(shopTables[0]?.key ?? "")
    expect(orders && customers && orders.x < customers.x).toBe(true)
    expect(layoutErd(shopTables, shopLinks)).toEqual(first)
  })
})
