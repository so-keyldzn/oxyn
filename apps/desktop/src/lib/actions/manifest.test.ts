import { describe, expect, it } from "vitest"

import { bindingsByZone, nativeAccelerators } from "./keyboard"
import {
  chordsOf,
  labelOf,
  manifest,
  onPlatform,
  placementOf,
  shortcutTexts,
} from "./manifest"
import type { Zone } from "./manifest"
import { menuBar } from "./menu-model"
import { behaviours } from "./registry"
import { forbiddenReason, parseChord, sameChord } from "./shortcut"
import type { Platform } from "./shortcut"

// The two tests ADR-0041 asks of the registry (point 1), and the rules of its
// points 2 and 3 checked on the whole manifest. A conflict never shows at run
// time — one of the two actions simply does not fire — so it must fail here.

const PLATFORMS: Array<Platform> = ["mac", "other"]

describe("the action registry", () => {
  it("gives every id of the manifest a behaviour, and nothing else one", () => {
    const declared = manifest.actions.map((spec) => spec.id).sort()
    expect(Object.keys(behaviours).sort()).toEqual(declared)
    expect(new Set(declared).size).toBe(declared.length)
  })

  it("reserves the plugin. prefix for declarative plugin actions", () => {
    expect(
      manifest.actions.filter((spec) => spec.id.startsWith("plugin."))
    ).toEqual([])
  })
})

describe.each(PLATFORMS)("the shortcuts on %s", (platform) => {
  const specs = manifest.actions.filter((spec) => onPlatform(spec, platform))

  it("parse, and use no forbidden combination", () => {
    for (const spec of specs) {
      for (const text of shortcutTexts(spec, platform)) {
        expect(() => parseChord(text, platform)).not.toThrow()
        expect(
          forbiddenReason(text, platform),
          `${spec.id}: ${text}`
        ).toBeNull()
      }
    }
  })

  it("are unique in each zone, a precise zone only masking the global level", () => {
    const bound = specs.flatMap((spec) =>
      chordsOf(spec, platform).map((chord) => ({ spec, chord }))
    )
    for (const [index, left] of bound.entries()) {
      for (const right of bound.slice(index + 1)) {
        if (!sameChord(left.chord, right.chord)) continue
        const zones: Array<Zone> = [left.spec.zone, right.spec.zone]
        const conflict =
          left.spec.zone === right.spec.zone ||
          zones.includes("app") ||
          !zones.includes("global")
        expect(
          conflict,
          `${left.spec.id} and ${right.spec.id} share a combination`
        ).toBe(false)
      }
    }
  })

  it("give no destructive action a shortcut", () => {
    for (const spec of specs.filter((candidate) => candidate.destructive))
      expect(chordsOf(spec, platform), spec.id).toEqual([])
  })

  it("select no tab by number: ⌃Tab and ⌃⇧Tab only", () => {
    const digits = specs.filter((spec) =>
      chordsOf(spec, platform).some((chord) => /^[0-9]$/.test(chord.key))
    )
    expect(digits.map((spec) => spec.id).sort()).toEqual([
      "preview.focusGrid",
      "view.catalog",
    ])
    const next = specs.find((spec) => spec.id === "tab.next")
    expect(next && shortcutTexts(next, platform)).toEqual(["Ctrl+Tab"])
  })
})

describe("the combination rules", () => {
  it("refuse Mod+Shift+<punctuation>, which ⇧ may be needed to type", () => {
    expect(forbiddenReason("Mod+Shift+/", "mac")).not.toBeNull()
    expect(forbiddenReason("Mod+Shift+/", "other")).not.toBeNull()
    expect(forbiddenReason("Mod+Shift+H", "other")).toBeNull()
  })

  it("refuse Ctrl+Alt+<printable> outside macOS, where it is AltGr", () => {
    expect(forbiddenReason("Mod+Alt+0", "other")).not.toBeNull()
    expect(forbiddenReason("Ctrl+Alt+B", "other")).not.toBeNull()
    expect(forbiddenReason("Mod+Alt+B", "mac")).toBeNull()
  })

  it("make ⌘⌥B Ctrl+Shift+B outside macOS", () => {
    const spec = manifest.actions.find(
      (action) => action.id === "view.sidePanel"
    )
    expect(spec && shortcutTexts(spec, "mac")).toEqual(["Mod+Alt+B"])
    expect(spec && shortcutTexts(spec, "other")).toEqual(["Mod+Shift+B"])
  })

  it("resolve ⌘/ by zone: a comment in the editor, the sheet elsewhere", () => {
    const zones = bindingsByZone(manifest, "mac")
    const slash = parseChord("Mod+/", "mac")
    const holder = (zone: Zone) =>
      zones.get(zone)?.find((binding) => sameChord(binding.chord, slash))?.id
    expect(holder("editor")).toBe("editor.toggleComment")
    expect(holder("global")).toBe("help.shortcuts")
  })
})

describe("the native macOS bar", () => {
  const native = nativeAccelerators(manifest)

  it("never binds Escape: Cancel is shown without it", () => {
    expect(native.has("console.cancel")).toBe(false)
    const cancel = manifest.actions.find((spec) => spec.id === "console.cancel")
    expect(cancel && placementOf(cancel, "mac")).not.toBeNull()
  })

  it("gives ⌘W to Close tab alone", () => {
    const commandW = parseChord("Mod+W", "mac")
    const holders = manifest.actions.filter(
      (spec) =>
        native.has(spec.id) &&
        chordsOf(spec, "mac").some((chord) => sameChord(chord, commandW))
    )
    expect(holders.map((spec) => spec.id)).toEqual(["tab.close"])
  })

  it("keeps Quit for Rust, with ⌘Q", () => {
    expect(native.has("app.quit")).toBe(true)
    const quit = manifest.actions.find((spec) => spec.id === "app.quit")
    expect(quit && labelOf(quit, "mac")).toBe("Quit Oxyn")
    expect(quit && labelOf(quit, "other")).toBe("Exit")
  })
})

describe("the web bar of Windows and Linux", () => {
  const menus = menuBar(manifest, "other")

  it("gives each menu a distinct mnemonic, and each entry one of its label", () => {
    const letters = menus.map((menu) => menu.mnemonic)
    expect(new Set(letters).size).toBe(letters.length)
    for (const menu of menus) {
      const entries = menu.groups.flat()
      const own = entries.map((entry) => entry.mnemonic)
      expect(new Set(own).size, menu.id).toBe(own.length)
      for (const entry of entries) {
        expect(entry.mnemonic, entry.id).not.toBeNull()
        expect(
          entry.labels[0]?.toLowerCase().includes(entry.mnemonic ?? "?"),
          entry.id
        ).toBe(true)
      }
    }
  })

  it("leaves Alt+<mnemonic> to the menus", () => {
    const mnemonics = new Set(menus.map((menu) => menu.mnemonic))
    for (const spec of manifest.actions.filter((action) =>
      onPlatform(action, "other")
    ))
      for (const chord of chordsOf(spec, "other"))
        expect(
          chord.alt && !chord.ctrl && !chord.shift && mnemonics.has(chord.key),
          spec.id
        ).toBe(false)
  })

  it("ends File with Exit, bound to Ctrl+Q", () => {
    const file = menus.find((menu) => menu.id === "file")
    const last = file?.groups.at(-1)?.at(-1)
    expect(last?.id).toBe("app.quit")
    expect(last?.labels[0]).toBe("Exit")
  })
})
