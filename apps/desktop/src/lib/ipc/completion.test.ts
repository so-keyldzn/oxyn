import { describe, expect, it } from "vitest"

import { CompletionCatalog, CompletionLoad } from "./completion"

// The shape `crates/oxyn-desktop/src/completion/tests.rs` serializes
// (`the_snapshot_crosses_in_camel_case_with_tagged_states`): a field renamed on
// one side only is named here, at the boundary, instead of read as `undefined`.
const crossing = {
  namespaces: [
    {
      name: "public",
      insert: "public",
      system: false,
      stale: false,
      listed: true,
      relations: [
        {
          name: "users",
          insert: "users",
          qualified: "public.users",
          kind: "table",
          stale: false,
          virtualTable: false,
          shadow: false,
          fields: {
            state: "loaded",
            items: [{ name: "id", insert: "id", dataType: "int8" }],
          },
        },
        {
          name: "order",
          insert: '"order"',
          qualified: 'public."order"',
          kind: "table",
          stale: true,
          virtualTable: false,
          shadow: false,
          fields: { state: "notLoaded" },
        },
        {
          name: "wide",
          insert: "wide",
          qualified: "public.wide",
          kind: "table",
          stale: false,
          virtualTable: false,
          shadow: false,
          fields: { state: "omitted", count: 3 },
        },
      ],
    },
  ],
  resolution: { state: "known", namespaces: ["public"], stale: false },
  folding: "lower",
  truncated: { namespacesLeftOut: 1, relationsLeftOut: 2, fieldsLeftOut: 3 },
}

describe("CompletionCatalog", () => {
  it("reads what the backend serializes, in its three column states", () => {
    const parsed = CompletionCatalog.parse(crossing)
    const states = parsed.namespaces[0]?.relations.map(
      (relation) => relation.fields.state
    )
    expect(states).toEqual(["loaded", "notLoaded", "omitted"])
    expect(parsed.truncated).toEqual({
      namespacesLeftOut: 1,
      relationsLeftOut: 2,
      fieldsLeftOut: 3,
    })
  })

  it("reads a root level with no name and an unknown resolution", () => {
    const parsed = CompletionCatalog.parse({
      namespaces: [
        {
          name: null,
          insert: null,
          system: false,
          stale: false,
          listed: false,
          relations: [],
        },
      ],
      resolution: { state: "unknown" },
      folding: "insensitive",
      truncated: null,
    })
    expect(parsed.namespaces[0]?.name).toBeNull()
    expect(parsed.resolution.state).toBe("unknown")
  })

  it("keeps an empty known path apart from an unknown one", () => {
    const parsed = CompletionCatalog.parse({
      namespaces: [],
      resolution: { state: "known", namespaces: [], stale: false },
      folding: "lower",
      truncated: null,
    })
    expect(parsed.resolution).toEqual({
      state: "known",
      namespaces: [],
      stale: false,
    })
  })

  it("refuses a folding rule it does not know rather than guess one", () => {
    expect(
      CompletionCatalog.safeParse({ ...crossing, folding: "upper" }).success
    ).toBe(false)
    expect(
      CompletionCatalog.safeParse({ ...crossing, folding: undefined }).success
    ).toBe(false)
  })

  it("refuses a column state it does not know rather than guess one", () => {
    const wrong = structuredClone(crossing)
    const relation = wrong.namespaces[0]?.relations[0]
    if (relation) relation.fields = { state: "partial" }
    expect(CompletionCatalog.safeParse(wrong).success).toBe(false)
  })
})

describe("CompletionLoad", () => {
  // `CompletionLoad` of `crates/oxyn-desktop/src/ipc/completion.rs`: counts, and
  // why the pass ended early, if it did.
  it("reads the counts of a pass and why it stopped", () => {
    expect(
      CompletionLoad.parse({ attempted: 4, failed: 1, stopped: "deadline" })
    ).toEqual({ attempted: 4, failed: 1, stopped: "deadline" })
    expect(
      CompletionLoad.parse({ attempted: 0, failed: 0, stopped: null }).stopped
    ).toBeNull()
  })

  it("refuses a count that is not one", () => {
    expect(
      CompletionLoad.safeParse({ attempted: -1, failed: 0, stopped: null })
        .success
    ).toBe(false)
  })
})
