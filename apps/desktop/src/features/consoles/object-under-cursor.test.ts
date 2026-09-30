import { describe, expect, it } from "vitest"

import { loadedObject, nameAt } from "./object-under-cursor"
import type { CatalogNode } from "@/lib/ipc/types"

function node(
  name: string,
  address: CatalogNode["address"],
  children: Array<CatalogNode> = [],
  system = false
): CatalogNode {
  return {
    address,
    name,
    kind: address.relation === null ? "schema" : "table",
    holdsRecords: address.relation !== null,
    system,
    comment: null,
    loaded: true,
    stale: false,
    children,
  }
}

const relation = (namespace: string, name: string, system = false) =>
  node(name, { catalog: null, namespace, relation: name }, [], system)

const tree = [
  node("public", { catalog: null, namespace: "public", relation: null }, [
    relation("public", "orders"),
    relation("public", "Order Lines"),
    relation("public", "pg_stat", true),
  ]),
  node("archive", { catalog: null, namespace: "archive", relation: null }, [
    relation("archive", "orders"),
    relation("archive", "invoices"),
  ]),
]

describe("the name under the cursor", () => {
  const sql = 'SELECT * FROM public.orders JOIN "Order Lines" ON 1 = 1'

  it("reads a dotted name, the cursor anywhere on it or right after it", () => {
    const at = sql.indexOf("orders")
    expect(nameAt(sql, at + 2)).toEqual([
      { text: "public", quoted: false },
      { text: "orders", quoted: false },
    ])
    expect(nameAt(sql, at + "orders".length)).toHaveLength(2)
  })

  it("reads a quoted name, doubled quotes included", () => {
    expect(nameAt(sql, sql.indexOf("Lines"))).toEqual([
      { text: "Order Lines", quoted: true },
    ])
    expect(nameAt('"a""b"', 2)).toEqual([{ text: 'a"b', quoted: true }])
  })

  it("names nothing on a space, a number or an operator", () => {
    expect(nameAt(sql, sql.indexOf(" = ") + 1)).toBeNull()
    expect(nameAt("SELECT 42", 8)).toBeNull()
  })
})

describe("MySQL names under the cursor", () => {
  it("keeps the qualifier when the schema has not been loaded", () => {
    const sql = "SELECT * FROM `archive`.`orders`"
    const parts = nameAt(sql, sql.indexOf("orders") + 2, "`")
    expect(parts).toEqual([
      { text: "archive", quoted: true },
      { text: "orders", quoted: true },
    ])
    expect(loadedObject([relation("shop", "orders")], parts ?? [])).toBeNull()
    expect(loadedObject(tree, parts ?? [])).toEqual({
      catalog: null,
      namespace: "archive",
      relation: "orders",
    })
  })

  it("keeps spaces and doubled backticks inside one name", () => {
    const sql = "SELECT * FROM `order lines`"
    expect(nameAt(sql, sql.indexOf("lines"), "`")).toEqual([
      { text: "order lines", quoted: true },
    ])
    expect(nameAt("`a``b`", 2, "`")).toEqual([{ text: "a`b", quoted: true }])
  })

  it.each([
    'WHERE status = "orders"',
    'WHERE status = "a""orders"',
    String.raw`WHERE status = "a\"orders"`,
    "WHERE status = 'orders'",
  ])("opens no object inside the string %s", (sql) => {
    expect(nameAt(sql, sql.indexOf("orders") + 2, "`")).toBeNull()
  })

  it("still reads the name after a string", () => {
    const sql = 'SELECT "orders" FROM `archive`.`orders`'
    expect(nameAt(sql, sql.lastIndexOf("orders") + 2, "`")).toEqual([
      { text: "archive", quoted: true },
      { text: "orders", quoted: true },
    ])
  })
})

describe.each(["PostgreSQL", "SQLite"])("%s identifier quoting", () => {
  it("keeps double-quoted qualified names, spaces and doubled quotes", () => {
    const sql = 'SELECT * FROM "archive"."Order Lines"'
    expect(nameAt(sql, sql.indexOf("Lines"), '"')).toEqual([
      { text: "archive", quoted: true },
      { text: "Order Lines", quoted: true },
    ])
    expect(nameAt('"a""b"', 2, '"')).toEqual([{ text: 'a"b', quoted: true }])
    expect(nameAt('"orders"', 2, '"')).toEqual([
      { text: "orders", quoted: true },
    ])
  })
})

describe("the loaded object a name resolves to", () => {
  it("resolves a qualified name, without case when unquoted", () => {
    expect(
      loadedObject(tree, [
        { text: "PUBLIC", quoted: false },
        { text: "Orders", quoted: false },
      ])
    ).toEqual({ catalog: null, namespace: "public", relation: "orders" })
  })

  it("opens nothing on an ambiguous name", () => {
    expect(loadedObject(tree, [{ text: "orders", quoted: false }])).toBeNull()
    expect(loadedObject(tree, [{ text: "invoices", quoted: false }])).toEqual({
      catalog: null,
      namespace: "archive",
      relation: "invoices",
    })
  })

  it("compares a quoted name exactly and leaves system objects out", () => {
    expect(
      loadedObject(tree, [{ text: "order lines", quoted: true }])
    ).toBeNull()
    expect(
      loadedObject(tree, [{ text: "Order Lines", quoted: true }])
    ).not.toBeNull()
    expect(loadedObject(tree, [{ text: "pg_stat", quoted: false }])).toBeNull()
  })
})
