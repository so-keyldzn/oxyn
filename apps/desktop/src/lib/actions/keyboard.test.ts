import { afterEach, describe, expect, it, vi } from "vitest"

import { NO_FOCUS, setZoneHandle } from "./context"
import type {
  ActionContext,
  ActionSources,
  ConsoleActions,
  WorkspaceActions,
} from "./context"
import { bindingsByZone, dispatch, nativeAccelerators } from "./keyboard"
import { manifest } from "./manifest"
import type { Platform } from "./shortcut"

// The dispatcher against the keyboards it must hold: ⌘ and Ctrl kept apart,
// AZERTY digits and punctuation, a Cyrillic layout, ⌥ on macOS, a CJK
// composition, a dialog over everything, and the native macOS bar that owns
// its combinations (docs/adr/0041-registre-d-actions-menus-et-raccourcis.md).

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

const console_: ConsoleActions = {
  run: nothing,
  runAll: nothing,
  explain: nothing,
  cancel: nothing,
  save: nothing,
}

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
    state: { canRun: true, running: false, cancelling: false, writing: false },
    actions: console_,
  },
  navigation: { state: {}, actions: { back: nothing } },
}

function press(
  platform: Platform,
  init: KeyboardEventInit,
  options: {
    target?: HTMLElement
    native?: boolean
    sources?: ActionSources
  } = {}
) {
  const target = options.target ?? document.body
  const invoked: Array<string> = []
  const context = (): ActionContext => ({
    ...NO_FOCUS,
    platform,
    sources: options.sources ?? SOURCES,
  })
  const listener = (event: Event) =>
    dispatch(event as KeyboardEvent, {
      nativeMenu: options.native ?? false,
      native: nativeAccelerators(manifest),
      bindings: bindingsByZone(manifest, platform),
      context,
      invoke: (id) => invoked.push(id),
    })
  target.addEventListener("keydown", listener)
  const event = new KeyboardEvent("keydown", {
    bubbles: true,
    cancelable: true,
    ...init,
  })
  const stop = vi.spyOn(event, "stopPropagation")
  target.dispatchEvent(event)
  target.removeEventListener("keydown", listener)
  return {
    invoked,
    prevented: event.defaultPrevented,
    stopped: stop.mock.calls.length > 0,
  }
}

function mount(html: string) {
  document.body.innerHTML = html
  return document.body
}

afterEach(() => {
  document.body.innerHTML = ""
})

describe("Mod", () => {
  it("is ⌘ on macOS, and ⌃ there is not ⌘", () => {
    expect(
      press("mac", { key: "t", code: "KeyT", metaKey: true }).invoked
    ).toEqual(["console.new"])
    expect(
      press("mac", { key: "t", code: "KeyT", ctrlKey: true }).invoked
    ).toEqual([])
  })

  it("is Ctrl elsewhere, and the Windows key is not Ctrl", () => {
    expect(
      press("other", { key: "t", code: "KeyT", ctrlKey: true }).invoked
    ).toEqual(["console.new"])
    expect(
      press("other", { key: "t", code: "KeyT", metaKey: true }).invoked
    ).toEqual([])
  })
})

describe("the identity of a key", () => {
  it("reads digits from event.code: ⌘1 on AZERTY produces &", () => {
    expect(
      press("mac", { key: "&", code: "Digit1", metaKey: true }).invoked
    ).toEqual(["view.catalog"])
    expect(
      press("other", { key: "&", code: "Digit1", ctrlKey: true }).invoked
    ).toEqual(["view.catalog"])
  })

  it("reads a punctuation key from event.key, ⇧ ignored: / is ⇧: on AZERTY", () => {
    mount('<div data-action-zone="editor"><textarea></textarea></div>')
    const zone = document.querySelector("[data-action-zone]")
    const field = document.querySelector("textarea")
    if (!zone || !(field instanceof HTMLElement)) throw new Error("no editor")
    setZoneHandle(zone, { toggleComment: nothing })
    const azerty = press(
      "mac",
      { key: "/", code: "Period", metaKey: true, shiftKey: true },
      { target: field }
    )
    expect(azerty.invoked).toEqual(["editor.toggleComment"])
  })

  it("falls back on event.code for a non-Latin layout", () => {
    expect(
      press("other", { key: "и", code: "KeyB", ctrlKey: true }).invoked
    ).toEqual(["view.sidebar"])
  })

  it("falls back on event.code when ⌥ changes the character on macOS", () => {
    expect(
      press("mac", { key: "∫", code: "KeyB", metaKey: true, altKey: true })
        .invoked
    ).toEqual(["view.sidePanel"])
    expect(
      press("other", { key: "B", code: "KeyB", ctrlKey: true, shiftKey: true })
        .invoked
    ).toEqual(["view.sidePanel"])
  })

  it("compares ⇧ for a letter", () => {
    expect(
      press("mac", { key: "T", code: "KeyT", metaKey: true, shiftKey: true })
        .invoked
    ).toEqual([])
  })
})

describe("what is never a shortcut", () => {
  it("ignores a key that validates a composition", () => {
    const composing = press("other", {
      key: "Enter",
      code: "Enter",
      ctrlKey: true,
      isComposing: true,
    })
    expect(composing.invoked).toEqual([])
    expect(composing.prevented).toBe(false)
    expect(
      press("other", {
        key: "Process",
        code: "Enter",
        ctrlKey: true,
        keyCode: 229,
      }).invoked
    ).toEqual([])
  })

  it("leaves Escape to the zone that owns it", () => {
    mount('<div data-action-zone="editor"><textarea></textarea></div>')
    const field = document.querySelector("textarea")
    if (!field) throw new Error("no field")
    const escape = press(
      "mac",
      { key: "Escape", code: "Escape" },
      { target: field }
    )
    expect(escape.invoked).toEqual([])
    expect(escape.prevented).toBe(false)
  })

  it("leaves Alt+← to a text field, where it moves by word", () => {
    mount("<input />")
    const field = document.querySelector("input")
    if (!field) throw new Error("no field")
    expect(
      press(
        "other",
        { key: "ArrowLeft", code: "ArrowLeft", altKey: true },
        { target: field }
      ).invoked
    ).toEqual([])
    expect(
      press("other", { key: "ArrowLeft", code: "ArrowLeft", altKey: true })
        .invoked
    ).toEqual(["nav.back"])
  })
})

describe("zones", () => {
  it("let a dialog hold the keyboard, Quit excepted", () => {
    mount('<div role="dialog"><button>OK</button></div>')
    const button = document.querySelector("button")
    if (!button) throw new Error("no button")
    expect(
      press(
        "other",
        { key: "t", code: "KeyT", ctrlKey: true },
        { target: button }
      ).invoked
    ).toEqual([])
    expect(
      press(
        "other",
        { key: "q", code: "KeyQ", ctrlKey: true },
        { target: button }
      ).invoked
    ).toEqual(["app.quit"])
  })

  it("send ⌘F to the zone with the focus", () => {
    mount(
      '<div data-action-zone="grid"><input data-find-field /><div role="grid" tabindex="0"></div></div>'
    )
    const grid = document.querySelector('[role="grid"]')
    if (!(grid instanceof HTMLElement)) throw new Error("no grid")
    expect(
      press("mac", { key: "f", code: "KeyF", metaKey: true }, { target: grid })
        .invoked
    ).toEqual(["edit.find"])
  })

  it("claim a greyed combination without running it", () => {
    const busy: ActionSources = {
      ...SOURCES,
      console: {
        state: {
          canRun: true,
          running: true,
          cancelling: false,
          writing: false,
        },
        actions: console_,
      },
    }
    const run = press(
      "other",
      { key: "Enter", code: "Enter", ctrlKey: true },
      { sources: busy }
    )
    expect(run.invoked).toEqual([])
    expect(run.prevented).toBe(true)
  })

  it("run and save nothing from a read-only editor, not even the active console", () => {
    mount(
      '<div data-action-zone="editor" data-read-only="true"><textarea></textarea></div>'
    )
    const field = document.querySelector("textarea")
    if (!field) throw new Error("no field")
    for (const key of [
      { key: "Enter", code: "Enter" },
      { key: "s", code: "KeyS" },
    ]) {
      const press_ = press(
        "other",
        { ...key, ctrlKey: true },
        { target: field }
      )
      expect(press_.invoked).toEqual([])
      expect(press_.prevented).toBe(true)
    }
  })

  it("let an absent action through", () => {
    const slash = press("mac", { key: "/", code: "Slash", metaKey: true })
    expect(slash.invoked).toEqual([])
    expect(slash.prevented).toBe(false)
  })
})

describe("the native macOS bar", () => {
  it("owns its combinations: the page neither runs nor prevents them", () => {
    const close = press(
      "mac",
      { key: "w", code: "KeyW", metaKey: true },
      { native: true }
    )
    expect(close.invoked).toEqual([])
    expect(close.prevented).toBe(false)
    // CodeMirror and the sidebar must not bind it a second time.
    expect(close.stopped).toBe(true)
  })

  it("leaves to the dispatcher what the bar does not bind", () => {
    expect(
      press("mac", { key: "Tab", code: "Tab", ctrlKey: true }, { native: true })
        .invoked
    ).toEqual(["tab.next"])
  })
})

describe("the context menu key", () => {
  it("opens the focused element's menu on ⇧F10 and the Menu key", () => {
    mount("<button>row</button>")
    const button = document.querySelector("button")
    if (!button) throw new Error("no button")
    expect(
      press(
        "other",
        { key: "F10", code: "F10", shiftKey: true },
        { target: button }
      ).invoked
    ).toEqual(["context-menu.open"])
    expect(
      press(
        "other",
        { key: "ContextMenu", code: "ContextMenu" },
        { target: button }
      ).invoked
    ).toEqual(["context-menu.open"])
  })
})
