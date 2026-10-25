import { describe, expect, it } from "vitest"

import { absentObjectTabs } from "./capabilities"
import { objectKindLabel } from "@/components/oxyn/object-view-frame"
import type { OpenConnection } from "@/lib/ipc/types"

// Only the capabilities are read: the rest of a connection does not matter.
const session = (capabilities: Array<string>) =>
  ({ capabilities }) as unknown as OpenConnection

describe("the object tabs a session draws", () => {
  it("draws every tab for a session that declares their capabilities", () => {
    const relational = session([
      "SQL",
      "INDEXES",
      "CONSTRAINTS",
      "FOREIGN_KEYS",
    ])
    expect([...absentObjectTabs(relational)]).toEqual([])
  })

  it("leaves out what a key-value session cannot fill, never Data or DDL", () => {
    const keyValue = session(["REDIS_COMMAND", "KEY_VALUE"])
    expect([...absentObjectTabs(keyValue)]).toEqual([
      "indexes",
      "constraints",
      "relations",
    ])
  })

  it("follows each flag on its own", () => {
    expect([...absentObjectTabs(session(["INDEXES"]))]).toEqual([
      "constraints",
      "relations",
    ])
  })

  it("draws Relations for either direction of keys", () => {
    expect([...absentObjectTabs(session(["INCOMING_FOREIGN_KEYS"]))]).toEqual([
      "indexes",
      "constraints",
    ])
  })
})

describe("the kind beside an object's name", () => {
  it("is written for reading", () => {
    expect(objectKindLabel("key_pattern")).toBe("key pattern")
    expect(objectKindLabel("materialized_view")).toBe("materialized view")
    expect(objectKindLabel("table")).toBe("table")
  })
})
