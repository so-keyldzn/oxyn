import { describe, expect, it } from "vitest"

import { HOSTILE_NAME, hidesText, operationOffer } from "./object-operations"
import type { CatalogNode } from "@/lib/ipc/types"

const node = (name: string, kind = "table"): CatalogNode => ({
  address: { catalog: null, namespace: "public", relation: name },
  name,
  kind,
  holdsRecords: true,
  system: false,
  comment: null,
  loaded: true,
  stale: false,
  children: [],
})

const POSTGRES = ["SQL", "DDL", "TRUNCATE", "TRANSACTIONAL_DDL"]
const SQLITE = ["SQL", "DDL", "TRANSACTIONAL_DDL"]
const READ_ONLY = ["SQL", "READ_ONLY_SESSION"]

describe("object operations offered from the catalog", () => {
  it("follows the declared capabilities, never the product", () => {
    for (const operation of ["drop", "truncate", "rename"] as const)
      expect(operationOffer(operation, node("orders"), POSTGRES)).toEqual({
        state: "offered",
      })
    expect(operationOffer("drop", node("orders"), SQLITE).state).toBe("offered")
    expect(operationOffer("rename", node("orders"), SQLITE).state).toBe(
      "offered"
    )
    expect(operationOffer("truncate", node("orders"), SQLITE)).toEqual({
      state: "greyed",
      reason: "This database has no TRUNCATE statement.",
    })
    for (const operation of ["drop", "truncate", "rename"] as const)
      expect(operationOffer(operation, node("orders"), READ_ONLY)).toEqual({
        state: "greyed",
        reason: "This connection does not accept schema changes.",
      })
  })

  it("offers each operation only on the kinds it applies to", () => {
    expect(operationOffer("drop", node("v", "view"), POSTGRES).state).toBe(
      "offered"
    )
    expect(
      operationOffer("drop", node("m", "materialized_view"), POSTGRES).state
    ).toBe("offered")
    expect(operationOffer("truncate", node("v", "view"), POSTGRES).state).toBe(
      "absent"
    )
    expect(operationOffer("rename", node("v", "view"), POSTGRES).state).toBe(
      "absent"
    )
    expect(operationOffer("drop", node("f", "function"), POSTGRES).state).toBe(
      "absent"
    )
    const schema = {
      ...node("public", "namespace"),
      address: { catalog: null, namespace: "public", relation: null },
    }
    expect(operationOffer("drop", schema, POSTGRES).state).toBe("absent")
  })

  it("greys a name holding control characters, in any segment", () => {
    for (const name of [
      "a\nb",
      "a\rb",
      "a\u2028b",
      "a\u2029b",
      "a\u202Eb",
      "a\u2067b",
      "a\u0000b",
    ]) {
      expect(hidesText(name)).toBe(true)
      expect(operationOffer("drop", node(name), POSTGRES)).toEqual({
        state: "greyed",
        reason: HOSTILE_NAME,
      })
    }
    const hostileSchema = node("orders")
    hostileSchema.address = { ...hostileSchema.address, namespace: "x\ny" }
    expect(operationOffer("rename", hostileSchema, POSTGRES).state).toBe(
      "greyed"
    )
    expect(hidesText('users"; DROP TABLE audit; --')).toBe(false)
  })
})
