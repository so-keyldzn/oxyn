import { describe, expect, it } from "vitest"

import { mentionChoices, readQuery } from "./mention-search"
import type { DocumentEntry } from "@/lib/ipc/library"
import type { CatalogSearchHit } from "@/lib/ipc/metadata"

function hit(
  name: string,
  overrides: Partial<CatalogSearchHit> = {}
): CatalogSearchHit {
  return {
    address: { catalog: null, namespace: "public", relation: name },
    name,
    kind: "table",
    holdsRecords: true,
    matched: "relationName",
    matchedFields: [],
    ...overrides,
  }
}

function saved(
  id: string,
  connection: string | null,
  isSaved = true
): DocumentEntry {
  return {
    id,
    title: `Query ${id}`,
    connection,
    connectionName: null,
    updatedAt: "2026-09-24T00:00:00Z",
    isSaved,
    isOpen: false,
    hasChanges: false,
    fromAgent: false,
  }
}

describe("the @ list", () => {
  it("keeps the backend's order and names a column as table.column", () => {
    const choices = mentionChoices(
      "c1",
      "ord",
      [
        hit("orders"),
        hit("order_lines", {
          matched: "fieldName",
          matchedFields: ["order_id", "ordered_at"],
        }),
        hit("orders_v", { kind: "view" }),
      ],
      []
    )
    expect(choices.map((choice) => [choice.kind, choice.label])).toEqual([
      ["table", "orders"],
      ["column", "order_lines.order_id"],
      ["column", "order_lines.ordered_at"],
      ["view", "orders_v"],
    ])
    // An address and a field, never the label to reparse.
    expect(choices[1]?.mention).toEqual({
      kind: "relation",
      address: { catalog: null, namespace: "public", relation: "order_lines" },
      field: "order_id",
    })
  })

  it("offers the columns already in the cache, even when the table matched", () => {
    // `orders` matched by its own name; `order_id` matched too, and was
    // dropped by the first version of this list.
    const choices = mentionChoices(
      "c1",
      "order",
      [hit("orders", { matchedFields: ["order_id"] })],
      []
    )
    expect(choices.map((choice) => choice.label)).toEqual([
      "orders",
      "orders.order_id",
    ])
  })

  it("reads table.column as the columns of the matching tables", () => {
    expect(readQuery("orders.st")).toEqual({
      search: "orders st",
      relation: "orders",
      column: "st",
    })
    const choices = mentionChoices(
      "c1",
      "orders.st",
      [
        hit("orders", { matchedFields: ["status", "order_id", "stamp"] }),
        hit("stock", { matchedFields: ["stored_at"] }),
      ],
      [saved("a", "c1")]
    )
    // Only columns, only of `orders`, only starting with `st`: no saved query.
    expect(choices.map((choice) => choice.label)).toEqual([
      "orders.status",
      "orders.stamp",
    ])
  })

  it("offers only the saved queries of this connection or of none", () => {
    const choices = mentionChoices(
      "c1",
      "q",
      [],
      [
        saved("a", "c1"),
        saved("b", "c2"),
        saved("c", null),
        saved("d", "c1", false),
      ]
    )
    expect(choices.map((choice) => choice.mention)).toEqual([
      { kind: "savedQuery", document: "a" },
      { kind: "savedQuery", document: "c" },
    ])
  })

  it("is bounded, and keeps room for saved queries under many objects", () => {
    const hits = Array.from({ length: 30 }, (_, index) => hit(`t${index}`))
    const choices = mentionChoices("c1", "t", hits, [saved("a", "c1")])
    expect(choices).toHaveLength(12)
    expect(choices.at(-1)?.kind).toBe("savedQuery")
  })

  it("skips a level that is not a relation", () => {
    const schema = hit("public", {
      address: { catalog: null, namespace: "public", relation: null },
      kind: "namespace",
    })
    expect(mentionChoices("c1", "pub", [schema], [])).toEqual([])
  })
})
