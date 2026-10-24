import { describe, expect, it } from "vitest"

import { mongoNameAt } from "./name-at"

/** `text` with `|` marking the cursor. */
function at(marked: string) {
  const offset = marked.indexOf("|")
  return mongoNameAt(marked.replace("|", ""), offset)
}

describe("mongoNameAt", () => {
  it("reads the collection a command names, under its database", () => {
    expect(at('{"find": "ord|ers", "$db": "shop"}')).toEqual([
      { text: "shop", quoted: true },
      { text: "orders", quoted: true },
    ])
  })

  it("takes the database of the command under the cursor", () => {
    expect(
      at(
        '{"find": "us|ers", "$db": "shop"}\n{"update": "orders", "$db": "warehouse"}'
      )
    ).toEqual([
      { text: "shop", quoted: true },
      { text: "users", quoted: true },
    ])
    expect(at('{"find": "users", "$db": "shop"}\n{"find": "ord|ers"}')).toEqual(
      [{ text: "orders", quoted: true }]
    )
  })

  it("reads a nested name, such as a $lookup's source", () => {
    expect(
      at(
        '{"aggregate": "orders", "pipeline": [{"$lookup": {"from": "it|ems"}}]}'
      )
    ).toEqual([{ text: "items", quoted: true }])
  })

  it("decodes the string's escapes", () => {
    expect(at('{"find": "a\\"|b"}')).toEqual([{ text: 'a"b', quoted: true }])
  })

  it("names nothing on a key, a number, a $db or an open string", () => {
    expect(at('{"fi|nd": "orders"}')).toBeNull()
    expect(at('{"find": "orders", "limit": 1|0}')).toBeNull()
    expect(at('{"find": "orders", "$db": "sh|op"}')).toBeNull()
    expect(at('{"find": "ord|ers')).toBeNull()
  })
})
