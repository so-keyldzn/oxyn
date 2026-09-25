import { describe, expect, it, vi } from "vitest"

import { moveShownColumn } from "./column-order"
import {
  dropIndex,
  dropMark,
  inOrder,
  moveItem,
  moveStep,
  trackPointerDrag,
} from "./pointer-drag"

describe("reordering by drag", () => {
  const centers = [50, 150, 250, 350]

  it("lands an item after every other center the pointer passed", () => {
    expect(dropIndex(centers, 0, 10)).toBe(0)
    expect(dropIndex(centers, 0, 200)).toBe(1)
    expect(dropIndex(centers, 0, 400)).toBe(3)
    expect(dropIndex(centers, 3, 100)).toBe(1)
    expect(dropIndex(centers, 3, 0)).toBe(0)
    // Its own center does not count: a small move lands where it was.
    expect(dropIndex(centers, 1, 160)).toBe(1)
  })

  it("moves an item, and copies the list for an index out of range", () => {
    expect(moveItem(["a", "b", "c", "d"], 0, 2)).toEqual(["b", "c", "a", "d"])
    expect(moveItem(["a", "b", "c", "d"], 3, 0)).toEqual(["d", "a", "b", "c"])
    expect(moveItem(["a", "b"], 0, 5)).toEqual(["a", "b"])
    expect(moveItem(["a", "b"], -1, 0)).toEqual(["a", "b"])
  })

  it("marks the side of the item the dragged one lands on", () => {
    expect(dropMark({ from: 3, to: 1 }, 1)).toBe("before")
    expect(dropMark({ from: 0, to: 2 }, 2)).toBe("after")
    expect(dropMark({ from: 0, to: 2 }, 1)).toBeNull()
    expect(dropMark({ from: 2, to: 2 }, 2)).toBeNull()
    expect(dropMark(null, 0)).toBeNull()
  })

  it("keeps the dragged order, new items last in their own order", () => {
    const tabs = ["object:a", "console:1", "console:2", "result:9"]
    expect(inOrder(tabs, (key) => key, [])).toEqual(tabs)
    expect(
      inOrder(tabs, (key) => key, ["console:2", "object:a", "gone"])
    ).toEqual(["console:2", "object:a", "console:1", "result:9"])
  })
})

describe("the button that drags", () => {
  const press = (init: MouseEventInit) =>
    new MouseEvent("pointerdown", { clientX: 0, clientY: 0, ...init })
  const moveFar = () =>
    window.dispatchEvent(new MouseEvent("pointermove", { clientX: 200 }))

  // A right click, or ⌃-click on macOS, opens the surface's context menu:
  // it must not also pick up the tab, the column or the name under it.
  it("is the primary one, without ⌃ or ⌘", () => {
    for (const init of [
      { button: 2 },
      { button: 1 },
      { button: 0, ctrlKey: true },
      { button: 0, metaKey: true },
    ]) {
      const onStart = vi.fn()
      trackPointerDrag(press(init) as PointerEvent, {
        onStart,
        onMove: vi.fn(),
        onDrop: vi.fn(),
        onCancel: vi.fn(),
      })
      moveFar()
      expect(onStart).not.toHaveBeenCalled()
    }
  })
})

describe("the keyboard's side of a drag", () => {
  const key = (init: KeyboardEventInit) => new KeyboardEvent("keydown", init)

  it("moves on ⌥⇧← and ⌥⇧→ only", () => {
    expect(
      moveStep(key({ key: "ArrowLeft", altKey: true, shiftKey: true }))
    ).toBe(-1)
    expect(
      moveStep(key({ key: "ArrowRight", altKey: true, shiftKey: true }))
    ).toBe(1)
    // ⇧← extends a selection, ⌥← is Back outside macOS: neither moves.
    expect(moveStep(key({ key: "ArrowLeft", shiftKey: true }))).toBe(0)
    expect(moveStep(key({ key: "ArrowLeft", altKey: true }))).toBe(0)
    expect(
      moveStep(
        key({ key: "ArrowLeft", altKey: true, shiftKey: true, metaKey: true })
      )
    ).toBe(0)
  })
})

describe("moved columns", () => {
  it("move among the shown ones, a hidden column keeping its place", () => {
    expect(moveShownColumn([0, 1, 2], [0, 1, 2], 0, 2)).toEqual([1, 2, 0])
    expect(moveShownColumn([0, 1, 2], [0, 1, 2], 2, 0)).toEqual([2, 0, 1])
    // Column 1 hidden: moving 0 one step right passes 2, not 1.
    expect(moveShownColumn([0, 1, 2, 3], [0, 2, 3], 0, 1)).toEqual([1, 2, 0, 3])
  })

  it("leave the order alone when nothing moves", () => {
    const order = [0, 1, 2]
    expect(moveShownColumn(order, [0, 1, 2], 1, 1)).toBe(order)
    expect(moveShownColumn(order, [0, 1, 2], 1, 7)).toBe(order)
  })
})
