import { afterEach, beforeEach, describe, expect, it } from "vitest"

import { suppressBrowserDefaults } from "./browser-defaults"

let release: () => void = () => {}

function mount(html: string, mac = true) {
  document.body.innerHTML = html
  release = suppressBrowserDefaults(window, mac)
}

afterEach(() => {
  release()
  document.body.innerHTML = ""
})

/** Dispatches `event` on the element `selector` names, and says whether the
 * browser's default survived it. */
function keepsDefault(selector: string, event: Event): boolean {
  const target = document.querySelector(selector)
  if (target === null) throw new Error(`no element ${selector}`)
  target.dispatchEvent(event)
  return !event.defaultPrevented
}

const contextMenu = () =>
  new MouseEvent("contextmenu", { bubbles: true, cancelable: true })

const key = (init: KeyboardEventInit) =>
  new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init })

describe("the page's context menu", () => {
  beforeEach(() =>
    mount(`
      <p id="label">Tables</p>
      <input id="text" />
      <input id="check" type="checkbox" />
      <textarea id="area"></textarea>
      <div id="editor" contenteditable="true"><span id="line">select</span></div>
      <div id="menu"><span id="row">users</span></div>
    `)
  )

  it("is gone from the interface frame", () => {
    expect(keepsDefault("#label", contextMenu())).toBe(false)
    expect(keepsDefault("#check", contextMenu())).toBe(false)
  })

  it("stays in a text field, for Cut, Copy and Paste", () => {
    expect(keepsDefault("#text", contextMenu())).toBe(true)
    expect(keepsDefault("#area", contextMenu())).toBe(true)
    expect(keepsDefault("#line", contextMenu())).toBe(true)
  })

  it("leaves a menu of Oxyn's to open", () => {
    const row = document.querySelector("#row")
    const own = (event: Event) => event.preventDefault()
    row?.addEventListener("contextmenu", own)
    const event = contextMenu()
    row?.dispatchEvent(event)
    // Cancelled once, by the menu that opens instead: nothing else runs.
    expect(event.defaultPrevented).toBe(true)
    row?.removeEventListener("contextmenu", own)
  })
})

describe("keys a browser binds", () => {
  beforeEach(() => mount(`<p id="frame">Oxyn</p><input id="text" />`))

  it.each([
    ["⌘R", { metaKey: true, key: "r", code: "KeyR" }],
    ["⌘⇧R", { metaKey: true, shiftKey: true, key: "R", code: "KeyR" }],
    ["Ctrl+R", { ctrlKey: true, key: "r", code: "KeyR" }],
    ["⌘R on a Cyrillic layout", { metaKey: true, key: "к", code: "KeyR" }],
    ["F5", { key: "F5", code: "F5" }],
    ["⌘+", { metaKey: true, key: "=", code: "Equal" }],
    ["⌘-", { metaKey: true, key: "-", code: "Minus" }],
    ["⌘0 on AZERTY", { metaKey: true, key: "à", code: "Digit0" }],
    ["Ctrl+numpad +", { ctrlKey: true, key: "+", code: "NumpadAdd" }],
    ["the Back key", { key: "BrowserBack" }],
  ])("%s does nothing, even in a field", (_name, init) => {
    expect(keepsDefault("#frame", key(init))).toBe(false)
    expect(keepsDefault("#text", key(init))).toBe(false)
  })

  it.each([
    ["R", { key: "r", code: "KeyR" }],
    ["0", { key: "0", code: "Digit0" }],
    ["⌘C", { metaKey: true, key: "c", code: "KeyC" }],
    ["⌘Enter", { metaKey: true, key: "Enter", code: "Enter" }],
  ])("%s keeps its meaning", (_name, init) => {
    expect(keepsDefault("#text", key(init))).toBe(true)
  })

  it("leaves a key being composed alone", () => {
    const composing = key({ metaKey: true, key: "r", code: "KeyR" })
    Object.defineProperty(composing, "isComposing", { value: true })
    expect(keepsDefault("#text", composing)).toBe(true)
  })

  it("keeps Alt+← for the caret under macOS", () => {
    const init = { altKey: true, key: "ArrowLeft", code: "ArrowLeft" }
    expect(keepsDefault("#text", key(init))).toBe(true)
  })
})

describe("back and forward elsewhere than macOS", () => {
  beforeEach(() => mount(`<p id="frame">Oxyn</p>`, false))

  it("Alt+← and Alt+→ do not go through history", () => {
    for (const arrow of ["ArrowLeft", "ArrowRight"])
      expect(
        keepsDefault("#frame", key({ altKey: true, key: arrow, code: arrow }))
      ).toBe(false)
  })
})

describe("the mouse and the trackpad", () => {
  beforeEach(() =>
    mount(`
      <p id="frame">Oxyn</p>
      <div data-own-zoom><div id="diagram">erd</div></div>
    `)
  )

  it.each([3, 4])("button %i does not go through history", (button) => {
    for (const type of ["mouseup", "auxclick"])
      expect(
        keepsDefault(
          "#frame",
          new MouseEvent(type, { bubbles: true, cancelable: true, button })
        )
      ).toBe(false)
  })

  it("a pinch does not zoom the page", () => {
    const pinch = new WheelEvent("wheel", {
      bubbles: true,
      cancelable: true,
      ctrlKey: true,
      deltaY: -4,
    })
    expect(keepsDefault("#frame", pinch)).toBe(false)
    const gesture = new Event("gesturestart", {
      bubbles: true,
      cancelable: true,
    })
    expect(keepsDefault("#frame", gesture)).toBe(false)
  })

  it("a pinch still reaches the diagram, which zooms itself", () => {
    const pinch = new WheelEvent("wheel", {
      bubbles: true,
      cancelable: true,
      ctrlKey: true,
      deltaY: -4,
    })
    expect(keepsDefault("#diagram", pinch)).toBe(true)
    const gesture = new Event("gesturechange", {
      bubbles: true,
      cancelable: true,
    })
    expect(keepsDefault("#diagram", gesture)).toBe(true)
  })

  it("a plain wheel still scrolls", () => {
    const wheel = new WheelEvent("wheel", {
      bubbles: true,
      cancelable: true,
      deltaY: 40,
    })
    expect(keepsDefault("#frame", wheel)).toBe(true)
  })
})

describe("release", () => {
  it("gives the defaults back", () => {
    mount(`<p id="frame">Oxyn</p>`)
    release()
    release = () => {}
    expect(keepsDefault("#frame", contextMenu())).toBe(true)
  })
})
