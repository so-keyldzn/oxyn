import { expect, it } from "vitest"
import { retainErdPositions, nodeHeight } from "./erd-layout"
import { shopTables } from "./erd-fixtures"

it("does not place a new neighbour over the retained original object", () => {
  const tables = shopTables.slice(0, 3)
  const previous = new Map([
    [tables[0]!.key, { x: 0, y: 0 }],
    [tables[1]!.key, { x: 0, y: 500 }],
  ])
  const proposed = new Map(tables.map((table) => [table.key, { x: 0, y: 0 }]))
  const placed = retainErdPositions(tables, proposed, previous)
  expect(placed.get(tables[0]!.key)).toEqual(previous.get(tables[0]!.key))
  expect(placed.get(tables[1]!.key)).toEqual(previous.get(tables[1]!.key))
  expect(placed.get(tables[2]!.key)?.y).toBeGreaterThanOrEqual(
    nodeHeight(tables[0]!)
  )
  expect(placed.get(tables[2]!.key)).not.toEqual(placed.get(tables[1]!.key))
})
it("lets a newly read taller description grow without obscuring its sibling", () => {
  const tables = shopTables.slice(0, 2)
  const previous = new Map([
    [tables[0]!.key, { x: 0, y: 0 }],
    [tables[1]!.key, { x: 0, y: 60 }],
  ])
  const placed = retainErdPositions(tables, new Map(), previous)
  expect(placed.get(tables[0]!.key)).toEqual({ x: 0, y: 0 })
  expect(placed.get(tables[1]!.key)?.y).toBeGreaterThanOrEqual(
    nodeHeight(tables[0]!)
  )
})
it("uses fresh placement for another owner even when display labels match", () => {
  const table = { ...shopTables[0]!, key: "new-owner" }
  const next = new Map([[table.key, { x: 500, y: 40 }]])
  expect(
    retainErdPositions([table], next, new Map([["old-owner", { x: 0, y: 0 }]]))
  ).toBe(next)
})
