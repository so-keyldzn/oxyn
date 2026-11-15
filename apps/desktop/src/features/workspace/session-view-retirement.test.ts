import { expect, it } from "vitest"
import { sessionViewRetirement } from "./session-view-retirement"

it("keeps root-qualified map→Data views when console entries change", () => {
  const views = [
    { key: "root-map", owner: { session: "catalog" } },
    { key: "root-map-data", owner: { session: "catalog" } },
    { key: "connection-object" },
    { key: "private-a", owner: { session: "console-a" } },
    { key: "private-b", owner: { session: "console-b" } },
  ]
  const before = structuredClone(views)
  const { live, retired } = sessionViewRetirement(
    "catalog",
    ["console-b"],
    views
  )
  expect(live.has("catalog")).toBe(true)
  expect([...retired]).toEqual(["private-a"])
  expect(views).toEqual(before)
})
it("retires only the handed-off or removed owner's object, preview and map identities", () => {
  const { retired } = sessionViewRetirement(
    "catalog",
    ["console-b"],
    [
      { key: "object-a", owner: { session: "console-a" } },
      { key: "preview-a", owner: { session: "console-a" } },
      { key: "map-a", owner: { session: "console-a" } },
      { key: "map-b", owner: { session: "console-b" } },
      { key: "root-map", owner: { session: "catalog" } },
    ]
  )
  expect([...retired]).toEqual(["object-a", "preview-a", "map-a"])
})
