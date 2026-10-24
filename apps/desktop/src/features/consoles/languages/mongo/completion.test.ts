import { CompletionContext } from "@codemirror/autocomplete"
import { EditorState } from "@codemirror/state"
import { describe, expect, it } from "vitest"

import { insertion, mongoCompletionSource, suggestionsAt } from "./completion"
import { mongoMode } from "./index"
import type {
  CompletionCatalog,
  CompletionNamespace,
  CompletionRelation,
} from "@/lib/ipc/completion"

// Legal in MongoDB: a collection name may hold a quote and a backslash.
const HOSTILE = 'orders"}, "$db": "admin\\'

function collection(
  name: string,
  fields: Array<string> | null = null
): CompletionRelation {
  return {
    name,
    insert: name,
    qualified: name,
    kind: "collection",
    stale: false,
    virtualTable: false,
    shadow: false,
    fields:
      fields === null
        ? { state: "notLoaded" }
        : {
            state: "loaded",
            items: fields.map((field) => ({
              name: field,
              insert: field,
              dataType: "string",
            })),
          },
  }
}

function database(
  name: string,
  relations: Array<CompletionRelation>,
  system = false
): CompletionNamespace {
  return { name, insert: name, system, stale: false, listed: true, relations }
}

const catalog: CompletionCatalog = {
  namespaces: [
    database("shop", [
      collection("orders", ["total", "status", "customer"]),
      collection(HOSTILE),
    ]),
    database("archive", [collection("orders_2025", ["year"])]),
    database("admin", [], true),
  ],
  resolution: { state: "known", namespaces: ["shop"], stale: false },
  folding: "lower",
  truncated: null,
}

/** The labels offered with the cursor where `|` is. */
function labels(marked: string, snapshot: CompletionCatalog | null = catalog) {
  const pos = marked.indexOf("|")
  const text = marked.slice(0, pos) + marked.slice(pos + 1)
  return suggestionsAt(text, pos, snapshot)?.options.map((one) => one.label)
}

describe("the command", () => {
  it("offers the command names as the first key, with what they do", () => {
    const pos = 1
    const found = suggestionsAt("{}", pos, null)
    const find = found?.options.find((option) => option.label === "find")
    expect(find).toMatchObject({ detail: "reads", type: "keyword" })
    expect(
      found?.options.find((option) => option.label === "delete")?.detail
    ).toBe("writes")
  })

  it("does not offer the operators that run JavaScript on the server", () => {
    const offered = [
      ...(labels('{"find": "orders", "filter": {"|"}}') ?? []),
      ...(labels(
        '{"aggregate": "orders", "pipeline": [{"$group": {"_id": 1, "n": {"|"}}}]}'
      ) ?? []),
    ]
    expect(offered).not.toContain("$where")
    expect(offered).not.toContain("$accumulator")
    expect(offered).not.toContain("$function")
  })

  it("offers the command's own fields after its name, those typed excepted", () => {
    const offered = labels('{"find": "orders", "limit": 5, "|"}')
    expect(offered).toContain("filter")
    expect(offered).toContain("projection")
    expect(offered).toContain("$db")
    expect(offered).not.toContain("limit")
    expect(offered).not.toContain("pipeline")
  })

  it("offers the collections of the database the names resolve in", () => {
    expect(labels('{"find": "|"}')).toEqual(["orders", HOSTILE])
  })

  it("follows a typed $db, wherever it is written", () => {
    expect(labels('{"find": "|", "$db": "archive"}')).toEqual(["orders_2025"])
  })

  it("offers no collection while the database is unknown", () => {
    const unresolved: CompletionCatalog = {
      ...catalog,
      resolution: { state: "unknown" },
    }
    expect(labels('{"find": "|"}', unresolved)).toEqual([])
    expect(labels('{"find": "|", "$db": "shop"}', unresolved)).toEqual([
      "orders",
      HOSTILE,
    ])
  })

  it("offers the databases as $db, the system ones last", () => {
    const pos = '{"find": "orders", "$db": "'.length
    const found = suggestionsAt('{"find": "orders", "$db": ""}', pos, catalog)
    expect(found?.options.map((option) => option.label)).toEqual([
      "shop",
      "archive",
      "admin",
    ])
    expect(found?.options.at(-1)).toMatchObject({ detail: "system", boost: -2 })
  })

  it("offers the vocabulary without the catalog, never a name", () => {
    expect(labels('{"find": "|"}', null)).toEqual([])
    expect(labels('{"find": "orders", "filter": {"|"}}', null)).toContain("$or")
  })

  it("reads an explained command as a command", () => {
    expect(labels('{"explain": {|}}')).toContain("aggregate")
    expect(labels('{"explain": {"find": "orders", "|"}}')).toContain("filter")
    expect(labels('{"explain": {"find": "orders"}, "|"}')).toEqual([
      "verbosity",
      "comment",
      "$db",
    ])
    expect(labels('{"explain": {"find": "orders"}, "verbosity": "|"}')).toEqual(
      ["queryPlanner", "executionStats", "allPlansExecution"]
    )
  })
})

describe("the query", () => {
  it("offers the fields of the collection and the logical operators", () => {
    const offered = labels('{"find": "orders", "filter": {"|"}}')
    expect(offered).toEqual(
      expect.arrayContaining(["total", "status", "$and", "$or", "$expr"])
    )
    expect(offered).not.toContain("$gt")
  })

  it("offers the operators of a field under it", () => {
    const offered = labels('{"find": "orders", "filter": {"total": {"$g|"}}}')
    expect(offered).toEqual(expect.arrayContaining(["$gt", "$gte", "$in"]))
    expect(offered).not.toContain("total")
  })

  it("follows $or and $not down to the predicate they hold", () => {
    expect(
      labels('{"find": "orders", "filter": {"$or": [{"status": 1}, {"|"}]}}')
    ).toContain("status")
    expect(
      labels('{"find": "orders", "filter": {"total": {"$not": {"|"}}}}')
    ).toContain("$gt")
  })

  it("reads the predicates of count, update and delete", () => {
    expect(labels('{"count": "orders", "query": {"|"}}')).toContain("total")
    expect(labels('{"update": "orders", "updates": [{"q": {"|"}}]}')).toContain(
      "status"
    )
    expect(
      labels('{"delete": "orders", "deletes": [{"q": {"total": {"|"}}}]}')
    ).toContain("$lt")
  })
})

describe("writes", () => {
  it("offers the fields of an update statement", () => {
    const offered = labels('{"update": "orders", "updates": [{"q": {}, "|"}]}')
    expect(offered).toEqual(
      expect.arrayContaining(["u", "upsert", "multi", "arrayFilters"])
    )
    expect(offered).not.toContain("q")
  })

  it("offers the update operators, then the fields under one", () => {
    expect(
      labels('{"update": "orders", "updates": [{"q": {}, "u": {"|"}}]}')
    ).toEqual(expect.arrayContaining(["$set", "$inc", "$unset"]))
    expect(
      labels('{"findAndModify": "orders", "update": {"$set": {"|"}}}')
    ).toContain("status")
  })

  it("offers the fields of an index specification", () => {
    expect(labels('{"createIndexes": "orders", "indexes": [{"|"}]}')).toEqual(
      expect.arrayContaining(["key", "name", "unique"])
    )
  })
})

describe("the pipeline", () => {
  it("offers the stages as the key of a stage, and only as its first", () => {
    const offered = labels('{"aggregate": "orders", "pipeline": [{"|"}]}')
    expect(offered).toEqual(
      expect.arrayContaining(["$match", "$group", "$lookup", "$out", "$merge"])
    )
    expect(
      labels('{"aggregate": "orders", "pipeline": [{"$limit": 5, "|"}]}')
    ).toEqual([])
  })

  it("reads $match as a query", () => {
    expect(
      labels('{"aggregate": "orders", "pipeline": [{"$match": {"|"}}]}')
    ).toContain("total")
  })

  it("offers _id, then the accumulators of an output field", () => {
    expect(
      labels('{"aggregate": "orders", "pipeline": [{"$group": {"|"}}]}')
    ).toEqual(["_id"])
    expect(
      labels(
        '{"aggregate": "orders", "pipeline": [{"$group": {"_id": "$status", "n": {"|"}}}]}'
      )
    ).toEqual(expect.arrayContaining(["$sum", "$avg", "$push"]))
  })

  it("offers field paths where a value starts with $", () => {
    expect(
      labels(
        '{"aggregate": "orders", "pipeline": [{"$group": {"_id": "$st|"}}]}'
      )
    ).toEqual(["$total", "$status", "$customer"])
  })

  it("offers collections where a stage names one", () => {
    expect(
      labels('{"aggregate": "orders", "pipeline": [{"$out": "|"}]}')
    ).toContain("orders")
    expect(
      labels(
        '{"aggregate": "orders", "pipeline": [{"$lookup": {"from": "|"}}]}'
      )
    ).toContain("orders")
  })

  it("follows $facet and $lookup down to their own pipelines", () => {
    expect(
      labels(
        '{"aggregate": "orders", "pipeline": [{"$facet": {"byStatus": [{"|"}]}}]}'
      )
    ).toContain("$sortByCount")
  })
})

describe("what is inserted", () => {
  const slot = (marked: string) => {
    const pos = marked.indexOf("|")
    const text = marked.slice(0, pos) + marked.slice(pos + 1)
    const found = suggestionsAt(text, pos, catalog)
    if (!found) throw new Error("a slot at the cursor")
    return found.slot
  }

  it("escapes a hostile name as JSON, inside a string or as a literal", () => {
    // Neither form can close the string and add a key of its own.
    expect(insertion(slot('{"find": "|"}'), HOSTILE)).toBe(
      'orders\\"}, \\"$db\\": \\"admin\\\\'
    )
    expect(insertion(slot('{"find": |}'), HOSTILE)).toBe(
      JSON.stringify(HOSTILE)
    )
    expect(JSON.parse(`{"find": ${JSON.stringify(HOSTILE)}}`)).toEqual({
      find: HOSTILE,
    })
  })

  it("writes a key typed bare as a key, colon included", () => {
    expect(insertion(slot('{"find": "orders", fil|}'), "filter")).toBe(
      '"filter": '
    )
  })

  it("closes a string left open", () => {
    expect(insertion(slot('{"find": "orders", "fil|\n}'), "filter")).toBe(
      'filter"'
    )
  })
})

describe("the editor source", () => {
  const complete = (marked: string, explicit = false) => {
    const pos = marked.indexOf("|")
    const state = EditorState.create({
      doc: marked.slice(0, pos) + marked.slice(pos + 1),
      extensions: [mongoMode({ catalog: () => catalog })],
    })
    return mongoCompletionSource(() => catalog)(
      new CompletionContext(state, pos, explicit)
    )
  }

  it("replaces the whole string the cursor is in", () => {
    const result = complete('{"find": "or|ds"}')
    expect(result).toMatchObject({ from: 10, to: 14 })
    expect(result?.options.find((one) => one.label === "orders")?.apply).toBe(
      "orders"
    )
  })

  it("opens between tokens only when asked", () => {
    expect(complete('{"find": "orders", |}')).toBeNull()
    expect(complete('{"find": "orders", |}', true)?.options).not.toHaveLength(0)
  })

  it("opens as soon as a quote is typed", () => {
    expect(complete('{"find": "orders", "|"}')?.options).not.toHaveLength(0)
  })
})
