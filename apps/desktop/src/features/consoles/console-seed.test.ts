import { describe, expect, it } from "vitest"

import { consoleFile } from "./languages"
import { consoleSeed, firstConsoleText } from "./use-consoles"

describe("the seed of a new console", () => {
  it("is an empty console, on the server's default context", () => {
    expect(consoleSeed({}, "doc-1", "Console 2")).toEqual({
      document: "doc-1",
      revision: 0,
      title: "Console 2",
      text: "",
      savedTitle: null,
      savedText: null,
      hasSavedCopy: false,
      fromAgent: false,
      parameters: [],
      needsValues: false,
      context: null,
    })
  })

  it("carries the schema of « New console on this schema », and no text", () => {
    const place = { catalog: "billing", namespace: "reporting" }
    const seed = consoleSeed({ context: place }, "doc-1", "Console 3")
    expect(seed.context).toEqual(place)
    // The context changes what the session resolves, never the text.
    expect(seed.text).toBe("")
  })
})

describe("the file a console's language gives it", () => {
  it("keeps SQL's name and starter", () => {
    expect(consoleFile("sql")).toEqual({
      extension: "sql",
      starter: "SELECT 1;",
    })
  })

  it("starts a Redis console empty: the language has no comment", () => {
    expect(consoleFile("redis")).toEqual({ extension: "redis", starter: "" })
  })

  it("starts a MongoDB console with a command that reads nothing", () => {
    expect(consoleFile("mongo")).toEqual({
      extension: "json",
      starter: '{"ping": 1}',
    })
  })

  it("gives any other language plain text, never SQL", () => {
    expect(consoleFile("cypher")).toEqual({ extension: "txt", starter: "" })
    expect(consoleFile(null)).toEqual({ extension: "txt", starter: "" })
  })
})

describe("the first console of a new workspace", () => {
  const draft = {
    text: "SELECT * FROM orders;",
    from: "billing",
    language: "sql" as const,
  }

  it("copies the draft of the same language, and says where from", () => {
    expect(firstConsoleText(draft, "sql")).toEqual({
      text: "SELECT * FROM orders;",
      notice:
        "Copied from billing, which keeps its console. Nothing was executed.",
    })
  })

  it("starts from its own starter when the draft is in another language", () => {
    expect(firstConsoleText(draft, "redis")).toEqual({ text: "", notice: null })
    expect(
      firstConsoleText({ text: "GET k", from: null, language: "redis" }, "sql")
    ).toEqual({ text: "SELECT 1;", notice: null })
  })
})
