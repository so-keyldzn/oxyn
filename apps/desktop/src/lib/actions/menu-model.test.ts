import { describe, expect, it } from "vitest"

import { NO_FOCUS } from "./context"
import type { ActionContext } from "./context"
import { Manifest, manifest } from "./manifest"
import type { Zone } from "./manifest"
import { entryStates, menuBar } from "./menu-model"

function context(zones: Array<Zone>, modal = false): ActionContext {
  return { ...NO_FOCUS, zones, modal, platform: "other", sources: {} }
}

describe("the menu model", () => {
  it("draws the same menus as the native bar, without macOS's own", () => {
    expect(menuBar(manifest, "other").map((menu) => menu.title)).toEqual([
      "File",
      "Edit",
      "View",
      "Query",
      "Help",
    ])
    expect(menuBar(manifest, "mac").map((menu) => menu.title)).toEqual([
      "Oxyn",
      "File",
      "Edit",
      "View",
      "Query",
      "Help",
    ])
  })

  it("takes the label variant of the zone with the focus", () => {
    const find = (zones: Array<Zone>) =>
      entryStates(manifest, "other", context(zones)).find(
        (entry) => entry.id === "edit.find"
      )?.variant
    expect(find([])).toBe(0)
    expect(find(["editor"])).toBe(1)
    expect(find(["grid"])).toBe(2)
  })

  it("shows a global entry without the combination a zone takes from it", () => {
    const masked = Manifest.parse({
      menus: [{ id: "help", title: "Help" }],
      actions: [
        {
          id: "help.sheet",
          label: "Sheet",
          zone: "global",
          shortcut: { other: "Mod+K" },
          menu: { other: { menu: "help", group: 0, order: 0 } },
        },
        {
          id: "editor.thing",
          label: "Thing",
          zone: "editor",
          shortcut: { other: "Mod+K" },
        },
      ],
    })
    const shortcut = (zones: Array<Zone>) =>
      entryStates(masked, "other", context(zones), () => true)[0]?.shortcut
    expect(shortcut([])).toBe(true)
    expect(shortcut(["editor"])).toBe(false)
  })

  it("moves ⌘/ from Keyboard shortcuts when the editor has the focus", () => {
    const sheet = (zones: Array<Zone>) =>
      entryStates(manifest, "mac", context(zones), () => true).find(
        (entry) => entry.id === "help.shortcuts"
      )?.shortcut
    expect(sheet([])).toBe(true)
    expect(sheet(["editor"])).toBe(false)
  })

  it("greys an entry with its reason under a dialog", () => {
    const run = entryStates(manifest, "other", context([], true)).find(
      (entry) => entry.id === "console.run"
    )
    expect(run?.availability).toEqual({ reason: "A dialog is open" })
  })
})
