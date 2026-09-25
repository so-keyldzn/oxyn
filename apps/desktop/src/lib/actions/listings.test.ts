import { afterEach, describe, expect, it } from "vitest"

import { NO_FOCUS, focusOf, setZoneHandle } from "./context"
import type {
  ActionContext,
  ActionSources,
  ConsoleActions,
  WorkspaceActions,
} from "./context"
import { bindingsByZone, dispatch, nativeAccelerators } from "./keyboard"
import { paletteSections, shortcutSheet } from "./listings"
import { chordsOf, labelOf, manifest, onPlatform } from "./manifest"
import { availability, behaviours } from "./registry"
import type { Platform } from "./shortcut"

// The palette and the shortcut sheet read the registry and the manifest, and
// nothing else (docs/adr/0041-registre-d-actions-menus-et-raccourcis.md,
// point 7): these tests hold them to it, and hold ⌘/ to its zone rule.

const nothing = () => undefined

const workspace: WorkspaceActions = {
  openConsole: nothing,
  closeActiveTab: nothing,
  nextTab: nothing,
  previousTab: nothing,
  showCatalog: nothing,
  showLibrary: nothing,
  focusPreview: nothing,
  focusConsole: nothing,
  toggleSidebar: nothing,
  toggleAside: nothing,
  openAssistant: nothing,
  switchConnection: nothing,
}

const consoleActions: ConsoleActions = {
  run: nothing,
  runAll: nothing,
  explain: nothing,
  cancel: nothing,
  save: nothing,
}

/** A workspace on a running console, with every trigger mounted. */
const SOURCES: ActionSources = {
  workspace: {
    state: {
      activeTab: "console:1",
      tabCount: 1,
      consoleCount: 1,
      objectActive: false,
      hasAside: true,
      hasAssistant: false,
    },
    actions: workspace,
  },
  console: {
    state: { canRun: true, running: true, cancelling: false, writing: false },
    actions: consoleActions,
  },
  catalog: {
    state: { connection: "connection" },
    actions: { openObject: nothing },
  },
  appearance: {
    state: { theme: "dark", density: "compact" },
    actions: { setTheme: nothing, setDensity: nothing },
  },
  overlays: {
    state: {},
    actions: {
      openPalette: nothing,
      openQuickOpen: nothing,
      openShortcuts: nothing,
    },
  },
  settings: { state: {}, actions: { open: nothing } },
}

function context(
  platform: Platform,
  sources: ActionSources = SOURCES,
  focus = NO_FOCUS
): ActionContext {
  return { ...focus, platform, sources }
}

const entries = (platform: Platform, at: ActionContext) =>
  paletteSections(manifest, platform, at).flatMap((section) => section.entries)

afterEach(() => {
  document.body.innerHTML = ""
})

describe.each<Platform>(["mac", "other"])("the palette on %s", (platform) => {
  it("lists exactly the registry's actions offered in the context, but itself", () => {
    const at = context(platform)
    const offered = manifest.actions
      .filter(
        (spec) =>
          onPlatform(spec, platform) &&
          spec.id !== "palette.open" &&
          availability(spec.id, at) !== "absent"
      )
      .map((spec) => spec.id)
      .sort()
    expect(
      entries(platform, at)
        .map((entry) => entry.id)
        .sort()
    ).toEqual(offered)
  })

  it("greys an entry with the registry's reason, and never another one", () => {
    const at = context(platform)
    for (const entry of entries(platform, at))
      expect(entry.availability, entry.id).toEqual(availability(entry.id, at))
    const run = entries(platform, at).find(
      (entry) => entry.id === "console.run"
    )
    expect(run?.availability).toEqual({ reason: "A query is running" })
  })

  it("leaves out what the context does not offer", () => {
    const bare = entries(platform, context(platform, {}))
    expect(bare.map((entry) => entry.id)).not.toContain("view.theme.dark")
    expect(bare.map((entry) => entry.id)).not.toContain("editor.toggleComment")
  })

  it("names a choice of a submenu after it, with its check mark", () => {
    const dark = entries(platform, context(platform)).find(
      (entry) => entry.id === "view.theme.dark"
    )
    expect(dark?.label).toBe("Theme: Dark")
    expect(dark?.section).toBe("View")
    expect(dark?.checked).toBe(true)
  })
})

describe("⌘/ by zone", () => {
  function editor() {
    document.body.innerHTML =
      '<div data-action-zone="editor"><textarea></textarea></div>'
    const zone = document.querySelector("[data-action-zone]")
    const field = document.querySelector("textarea")
    if (!zone || !(field instanceof HTMLElement)) throw new Error("no editor")
    setZoneHandle(zone, { toggleComment: nothing })
    return field
  }

  function press(target: HTMLElement) {
    const invoked: Array<string> = []
    const listener = (event: Event) =>
      dispatch(event as KeyboardEvent, {
        nativeMenu: false,
        native: nativeAccelerators(manifest),
        bindings: bindingsByZone(manifest, "other"),
        context: () => context("other"),
        invoke: (id) => invoked.push(id),
      })
    target.addEventListener("keydown", listener)
    target.dispatchEvent(
      new KeyboardEvent("keydown", {
        bubbles: true,
        cancelable: true,
        key: "/",
        code: "Slash",
        ctrlKey: true,
      })
    )
    target.removeEventListener("keydown", listener)
    return invoked
  }

  it("comments in the SQL editor", () => {
    expect(press(editor())).toEqual(["editor.toggleComment"])
  })

  it("opens the shortcut sheet anywhere else", () => {
    expect(press(document.body)).toEqual(["help.shortcuts"])
  })

  it("shows the sheet's ⌘/ in the palette only outside the editor", () => {
    const field = editor()
    const inEditor = entries("mac", context("mac", SOURCES, focusOf(field)))
    const sheet = inEditor.find((entry) => entry.id === "help.shortcuts")
    expect(sheet?.keys).toEqual([])
    expect(
      inEditor.find((entry) => entry.id === "editor.toggleComment")?.keys
    ).toEqual(["⌘", "/"])
    const elsewhere = entries("mac", context("mac"))
    expect(
      elsewhere.find((entry) => entry.id === "help.shortcuts")?.keys
    ).toEqual(["⌘", "/"])
  })
})

describe("⌘P", () => {
  it("is greyed, not absent, without a catalog: Ctrl+P must not print", () => {
    const noCatalog = { ...SOURCES, catalog: undefined }
    expect(
      availability("object.quickOpen", context("other", noCatalog))
    ).toEqual({ reason: "This source has no catalog to search" })
    const invoked: Array<string> = []
    const event = new KeyboardEvent("keydown", {
      cancelable: true,
      key: "p",
      code: "KeyP",
      ctrlKey: true,
    })
    dispatch(event, {
      nativeMenu: false,
      native: nativeAccelerators(manifest),
      bindings: bindingsByZone(manifest, "other"),
      context: () => context("other", noCatalog),
      invoke: (id) => invoked.push(id),
    })
    expect(invoked).toEqual([])
    expect(event.defaultPrevented).toBe(true)
  })

  it("is absorbed under a dialog too", () => {
    document.body.innerHTML = '<div role="dialog"><input /></div>'
    const field = document.querySelector("input")
    if (!field) throw new Error("no field")
    const event = new KeyboardEvent("keydown", {
      bubbles: true,
      cancelable: true,
      key: "p",
      code: "KeyP",
      ctrlKey: true,
    })
    const invoked: Array<string> = []
    const listener = (pressed: Event) =>
      dispatch(pressed as KeyboardEvent, {
        nativeMenu: false,
        native: nativeAccelerators(manifest),
        bindings: bindingsByZone(manifest, "other"),
        context: () => context("other"),
        invoke: (id) => invoked.push(id),
      })
    field.addEventListener("keydown", listener)
    field.dispatchEvent(event)
    field.removeEventListener("keydown", listener)
    expect(invoked).toEqual([])
    expect(event.defaultPrevented).toBe(true)
  })
})

describe("the shortcut sheet", () => {
  it.each<Platform>(["mac", "other"])(
    "lists every combination of the manifest on %s, once",
    (platform) => {
      const rows = shortcutSheet(manifest, platform).flatMap(
        (section) => section.rows
      )
      const bound = manifest.actions.filter(
        (spec) =>
          onPlatform(spec, platform) && chordsOf(spec, platform).length > 0
      )
      expect(rows.map((row) => row.id).sort()).toEqual(
        bound.map((spec) => spec.id).sort()
      )
      for (const row of rows) {
        const spec = bound.find((candidate) => candidate.id === row.id)
        expect(row.label).toBe(spec && labelOf(spec, platform))
      }
    }
  )

  it("puts Toggle comment under the SQL editor, the sheet itself everywhere", () => {
    const sections = shortcutSheet(manifest, "mac")
    const holder = (id: string) =>
      sections.find((section) => section.rows.some((row) => row.id === id))
        ?.title
    expect(holder("editor.toggleComment")).toBe("SQL editor")
    expect(holder("help.shortcuts")).toBe("Everywhere")
  })
})

describe("the new entries of the bar", () => {
  it("grey New window, Documentation and Format with their reason", () => {
    for (const id of ["window.new", "help.documentation", "console.format"])
      expect(behaviours[id]?.enabled(context("other")), id).toMatchObject({
        reason: expect.any(String),
      })
  })

  it("grey Export… when the tab on screen has no result", () => {
    expect(availability("result.export", context("other"))).toEqual({
      reason: "The active tab has no result to export",
    })
    document.body.innerHTML =
      '<div hidden><button data-action-export="">Export</button></div>' +
      '<button data-action-export="Nothing to export">Export</button>'
    expect(availability("result.export", context("other"))).toEqual({
      reason: "Nothing to export",
    })
  })
})
