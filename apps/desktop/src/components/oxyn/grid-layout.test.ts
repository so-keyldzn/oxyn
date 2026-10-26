import { describe, expect, it } from "vitest"

import { gridLayoutStore } from "./grid-layout"
import type { GridLayout } from "./grid-layout"
import type { ResultColumn } from "@/lib/ipc/types"

const columns: Array<ResultColumn> = [
  { name: "id", dataType: "Int64", nullable: false },
  { name: "customer", dataType: "Utf8", nullable: false },
]

const widened: GridLayout = {
  resized: { 1: 240 },
  estimated: { charWidth: 7, nullText: "NULL", widths: [60, 120] },
  order: [1, 0],
}

describe("the layout kept for a grid remounted over the same result", () => {
  it("is empty until a grid writes one", () => {
    expect(gridLayoutStore().read("res-1", columns)).toBeNull()
  })

  it("gives back what was written for the same result and columns", () => {
    const store = gridLayoutStore()
    store.write("res-1", columns, widened)
    expect(store.read("res-1", columns)).toEqual(widened)
    // Another array with the same columns is the same schema.
    const same = columns.map((column) => ({ ...column }))
    expect(store.read("res-1", same)).toBe(widened)
  })

  it("is not another result's", () => {
    const store = gridLayoutStore()
    store.write("res-1", columns, widened)
    expect(store.read("res-2", columns)).toBeNull()
  })

  it("is not the same result's once its columns differ", () => {
    const store = gridLayoutStore()
    store.write("res-1", columns, widened)
    const [id, customer] = columns
    if (!id || !customer) throw new Error("two columns")
    expect(store.read("res-1", [id])).toBeNull()
    expect(store.read("res-1", [customer, id])).toBeNull()
    expect(
      store.read("res-1", [id, { ...customer, name: "client" }])
    ).toBeNull()
    expect(store.read("res-1", [id, { ...customer, dataType: "Int32" }])).toBe(
      null
    )
  })

  it("keeps one result only: writing another forgets the first", () => {
    const store = gridLayoutStore()
    store.write("res-1", columns, widened)
    store.write("res-2", columns, { resized: {}, estimated: null, order: null })
    expect(store.read("res-1", columns)).toBeNull()
    expect(store.read("res-2", columns)).toEqual({
      resized: {},
      estimated: null,
      order: null,
    })
  })

  it("is each view's own", () => {
    const one = gridLayoutStore()
    one.write("res-1", columns, widened)
    expect(gridLayoutStore().read("res-1", columns)).toBeNull()
  })
})
