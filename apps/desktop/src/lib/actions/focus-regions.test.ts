import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import { cycleFocus, rememberRegionFocus, setFocusEntry } from "./focus-regions"

function mount() {
  document.body.innerHTML = `<main data-focus-workspace>
    <section data-focus-region="navigation"><button id="catalog">Catalog</button><button id="selected">Selected object</button></section>
    <section data-focus-region="editor"><textarea id="editor">select 1</textarea></section>
    <section data-focus-region="results"><button id="grid">Grid</button></section>
    <section data-focus-region="aside" hidden><button id="aside">Inspector</button></section>
  </main><div inert><main data-focus-workspace><section data-focus-region="editor"><textarea id="other">other base</textarea></section></main></div>`
}
const element = (id: string) => document.getElementById(id)!
beforeEach(() => {
  // jsdom has no layout. Visibility exclusions still read actual ancestor state.
  vi.spyOn(HTMLElement.prototype, "getClientRects").mockReturnValue([
    { width: 100, height: 40 },
  ] as unknown as DOMRectList)
  mount()
})
afterEach(() => {
  document.body.replaceChildren()
  vi.restoreAllMocks()
})
describe("workspace focus regions", () => {
  it("cycles in both directions through visible regions, excluding closed and foreign workspaces", () => {
    element("catalog").focus()
    cycleFocus(document.activeElement, 1)
    expect(document.activeElement).toBe(element("editor"))
    cycleFocus(document.activeElement, 1)
    expect(document.activeElement).toBe(element("grid"))
    cycleFocus(document.activeElement, 1)
    expect(document.activeElement).toBe(element("catalog"))
    cycleFocus(document.activeElement, -1)
    expect(document.activeElement).toBe(element("grid"))
  })
  it("discards a stale hidden-workspace context and enters the visible owner", () => {
    cycleFocus(element("other"), 1)
    expect(document.activeElement).toBe(element("catalog"))
  })

  it("restores the last element and its selection without editing the draft", () => {
    const editor = element("editor") as HTMLTextAreaElement
    editor.focus()
    editor.setSelectionRange(1, 5)
    rememberRegionFocus(editor)
    cycleFocus(editor, 1)
    cycleFocus(document.activeElement, -1)
    expect(document.activeElement).toBe(editor)
    expect([editor.selectionStart, editor.selectionEnd, editor.value]).toEqual([
      1,
      5,
      "select 1",
    ])
    element("selected").focus()
    rememberRegionFocus(element("selected"))
    cycleFocus(document.activeElement, 1)
    cycleFocus(document.activeElement, -1)
    expect(document.activeElement).toBe(element("selected"))
  })
  it("recomputes optional regions when mounted or removed and skips collapsed navigation", () => {
    const root = document.querySelector("main")!
    root.insertAdjacentHTML(
      "beforeend",
      '<section data-focus-region="map"><button id="map">Map</button></section>'
    )
    element("grid").focus()
    cycleFocus(document.activeElement, 1)
    expect(document.activeElement).toBe(element("map"))
    document
      .querySelector('[data-focus-region="navigation"]')!
      .setAttribute("data-focus-disabled", "true")
    cycleFocus(document.activeElement, 1)
    expect(document.activeElement).toBe(element("editor"))
  })
  it("returns to the selected tab when the previously focused tab was deactivated", () => {
    const region = document.querySelector('[data-focus-region="navigation"]')!
    region.innerHTML =
      '<button id="old-tab" role="tab" aria-selected="true">Old</button><button id="current-tab" role="tab" aria-selected="false" tabindex="-1">Current</button>'
    rememberRegionFocus(element("old-tab"))
    element("old-tab").setAttribute("aria-selected", "false")
    element("old-tab").tabIndex = -1
    element("current-tab").setAttribute("aria-selected", "true")
    element("current-tab").tabIndex = 0
    element("editor").focus()
    cycleFocus(document.activeElement, -1)
    expect(document.activeElement).toBe(element("current-tab"))
  })

  it("lets a modal hold focus even when the caller retained an older context", () => {
    document.body.insertAdjacentHTML(
      "beforeend",
      '<div role="dialog"><button id="cancel">Cancel</button></div>'
    )
    element("cancel").focus()
    expect(cycleFocus(element("catalog"), 1)).toBe(false)
    expect(document.activeElement).toBe(element("cancel"))
  })
  it("uses an editor engine's restoration entry and ignores detached remembered elements", () => {
    const restore = vi.fn(() => element("editor").focus())
    setFocusEntry(element("editor"), restore)
    element("catalog").focus()
    cycleFocus(document.activeElement, 1)
    expect(restore).toHaveBeenCalledTimes(1)
    rememberRegionFocus(element("selected"))
    element("selected").remove()
    cycleFocus(document.activeElement, -1)
    expect(document.activeElement).toBe(element("catalog"))
  })
})
