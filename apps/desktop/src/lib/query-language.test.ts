import { describe, expect, it } from "vitest"

import { queryLanguageOf } from "./query-language"

describe("queryLanguageOf", () => {
  it("speaks SQL when the session declares it, whatever else it declares", () => {
    expect(queryLanguageOf(["SQL", "TRANSACTIONS"])).toBe("sql")
    expect(queryLanguageOf(["REDIS_COMMAND", "SQL"])).toBe("sql")
  })

  it("speaks the one language a session without SQL declares", () => {
    expect(queryLanguageOf(["REDIS_COMMAND", "STREAMING"])).toBe("redis")
    expect(queryLanguageOf(["MONGO_QUERY"])).toBe("mongo")
    expect(queryLanguageOf(["SEARCH_DSL"])).toBe("search-dsl")
  })

  it("does not guess between two languages besides SQL", () => {
    expect(queryLanguageOf(["CYPHER", "GREMLIN"])).toBeNull()
  })

  it("has no language for a session that declares none", () => {
    expect(queryLanguageOf([])).toBeNull()
    expect(queryLanguageOf(["TRANSACTIONS", "TABLES"])).toBeNull()
  })
})
