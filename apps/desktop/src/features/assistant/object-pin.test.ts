import { describe, expect, it } from "vitest"

import { getPinState, pinObject, unpin } from "./object-pin"
import { canPin } from "@/components/oxyn/catalog-tree"
import type { PinToQuestion } from "@/components/oxyn/catalog-tree"
import type { CatalogNode } from "@/lib/ipc/types"

function relation(name: string, holdsRecords = true): CatalogNode {
  return {
    address: { catalog: null, namespace: "main", relation: name },
    name,
    kind: "table",
    holdsRecords,
    system: false,
    comment: null,
    loaded: true,
    stale: false,
    children: [],
  }
}

const offered: PinToQuestion = {
  tier: "sampled",
  destination: "provider",
  onPin: () => undefined,
}

describe("pinning an object to the question", () => {
  it("keeps one object: a question carries one sample", () => {
    pinObject("pins-1", relation("customers"))
    pinObject("pins-1", relation("invoices"))
    expect(getPinState("pins-1").pin).toMatchObject({
      label: "main.invoices",
      address: { relation: "invoices" },
    })
    unpin("pins-1")
    expect(getPinState("pins-1").pin).toBeNull()
  })

  it("is offered only where a sample can follow", () => {
    expect(canPin(offered, relation("customers"))).toBe(true)
    expect(canPin({ ...offered, tier: "metadata" }, relation("c"))).toBe(false)
    expect(canPin({ ...offered, tier: "local" }, relation("c"))).toBe(false)
    expect(canPin({ ...offered, destination: "agent" }, relation("c"))).toBe(
      false
    )
    expect(canPin({ ...offered, destination: null }, relation("c"))).toBe(false)
    // A sample is rows: an object that holds none has nothing to offer.
    expect(canPin(offered, relation("f", false))).toBe(false)
    expect(canPin(undefined, relation("c"))).toBe(false)
  })
})
