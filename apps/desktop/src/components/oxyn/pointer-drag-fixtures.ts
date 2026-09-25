// Drags for stories: a press on an element, moves on the window, a release —
// the events `trackPointerDrag` listens to, in the order a mouse sends them.

const POINTER = 7

function pointer(type: string, x: number, y: number) {
  return new PointerEvent(type, {
    bubbles: true,
    cancelable: true,
    composed: true,
    clientX: x,
    clientY: y,
    button: 0,
    buttons: type === "pointerup" ? 0 : 1,
    pointerId: POINTER,
    pointerType: "mouse",
    isPrimary: true,
  })
}

/** The center of `element`, in the viewport. */
export function centerOf(element: Element) {
  const box = element.getBoundingClientRect()
  return { x: box.left + box.width / 2, y: box.top + box.height / 2 }
}

/**
 * Presses `element` and moves to `to` in a few steps. The release is left to
 * `release`, so a story can look at the drag while it is under way.
 */
export function pressAndMove(element: Element, to: { x: number; y: number }) {
  const from = centerOf(element)
  element.dispatchEvent(pointer("pointerdown", from.x, from.y))
  for (const step of [0.25, 0.5, 0.75, 1]) {
    window.dispatchEvent(
      pointer(
        "pointermove",
        from.x + (to.x - from.x) * step,
        from.y + (to.y - from.y) * step
      )
    )
  }
}

/** Releases the pointer at `at`, ending the drag. */
export function release(element: Element, at: { x: number; y: number }) {
  element.dispatchEvent(pointer("pointerup", at.x, at.y))
}

/** A whole drag of `element` to `to`. */
export function drag(element: Element, to: { x: number; y: number }) {
  pressAndMove(element, to)
  release(element, to)
}
