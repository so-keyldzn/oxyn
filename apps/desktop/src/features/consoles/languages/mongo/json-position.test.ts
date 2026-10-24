import { describe, expect, it } from "vitest"

import { readJson } from "./json-position"

/** Reads `text` with the cursor where `|` is. */
function at(marked: string) {
  const pos = marked.indexOf("|")
  return readJson(marked.slice(0, pos) + marked.slice(pos + 1), pos)
}

describe("readJson", () => {
  it("reads the first key of an empty command", () => {
    const { slot } = at("{|}")
    expect(slot).toMatchObject({
      role: "key",
      quoted: false,
      typed: "",
      route: [],
      position: 0,
    })
  })

  it("reads a key being typed inside its quotes, up to the closing one", () => {
    const text = '{"fi|nd": "users"}'
    const { slot } = at(text)
    expect(slot).toMatchObject({
      role: "key",
      quoted: true,
      closed: true,
      typed: "fi",
      from: 2,
      to: 6,
      position: 0,
    })
  })

  it("ends an unclosed string with its line", () => {
    const { slot, rootKeys } = at('{"find": "users",\n "fil|\n}')
    expect(slot).toMatchObject({
      role: "key",
      quoted: true,
      closed: false,
      typed: "fil",
      position: 1,
      siblings: ["find"],
    })
    expect(rootKeys).toEqual(["find", "fil"])
  })

  it("reads a bare word as the key it starts", () => {
    const { slot } = at('{"find": "users", "filter": {$g|}}')
    expect(slot).toMatchObject({
      role: "key",
      quoted: false,
      typed: "$g",
      route: ["filter"],
    })
  })

  it("follows keys and array indexes down to the cursor", () => {
    const { slot } = at(
      '{"aggregate": "orders", "pipeline": [{"$match": {}}, {"$group": {"_id": "$c", "n": {"|"}}}]}'
    )
    expect(slot).toMatchObject({
      role: "key",
      route: ["pipeline", 1, "$group", "n"],
      heads: ["aggregate", null, "$group", "_id", null],
    })
  })

  it("names the key a value belongs to", () => {
    const { slot } = at('{"find": "us|"}')
    expect(slot).toMatchObject({ role: "value", key: "find", typed: "us" })
  })

  it("knows the root's strings, those after the cursor included", () => {
    const { rootKeys, rootStrings } = at(
      '{"find": "users", "filter": {|}, "$db": "shop"}'
    )
    expect(rootKeys).toEqual(["find", "filter", "$db"])
    expect(rootStrings.get("find")).toBe("users")
    expect(rootStrings.get("$db")).toBe("shop")
  })

  it("lists the container's other keys, those after the cursor included", () => {
    const { slot } = at('{"find": "users", "|", "limit": 5}')
    expect(slot?.siblings).toEqual(["find", "limit"])
  })

  it("decodes escaped keys and values", () => {
    const { rootStrings } = at('{"find": "a\\"b\\\\c", "x": |}')
    expect(rootStrings.get("find")).toBe('a"b\\c')
  })

  it("completes nothing where a colon or a comma is expected", () => {
    expect(at('{"find"| "users"}').slot).toBeNull()
    expect(at('{"find": "users"| }').slot).toBeNull()
  })

  it("completes nothing outside the command", () => {
    expect(at("|{}").slot).toBeNull()
    expect(at("{}|").slot).toBeNull()
  })

  it("survives a closing bracket that matches nothing", () => {
    const { slot } = at('{"find": "users"]], "|"}')
    expect(slot).toMatchObject({ role: "key", route: [] })
  })
})
