// Story helper: nothing shown here reaches the application.

import { expect, waitFor } from "storybook/test"

// Sub-pixel rounding of transforms and borders, not a real overflow.
const TOLERANCE = 0.5

/** Enough of an element to find it again from a failure message. */
function describe(element: Element) {
  const slot = element.getAttribute("data-slot")
  const text = element.textContent.trim().slice(0, 40)
  return `<${element.tagName.toLowerCase()}${slot ? ` data-slot="${slot}"` : ""}> ${text}`
}

/**
 * Whether an ancestor between `element` and `frame` clips it sideways — a
 * scroller or a truncation. What it hides is not drawn past the frame; the
 * clipping ancestor itself is measured like any other descendant.
 */
function clippedWithin(element: Element, frame: Element) {
  for (
    let parent = element.parentElement;
    parent && parent !== frame;
    parent = parent.parentElement
  ) {
    if (getComputedStyle(parent).overflowX !== "visible") return true
  }
  return false
}

/**
 * The box drawn in the window, and nothing inside it drawn past its edges.
 *
 * Nothing in the type checker, the linter or axe sees a dialog whose content
 * is wider than its frame: a grid track sized by its widest child, a
 * `shrink-0` button, a `min-w` that beats the box's `max-w` — each one draws a
 * list or a footer past the border, and only a screenshot shows it. This
 * measures what the screenshot would: every descendant's rectangle against the
 * box's, the box against the window, and the box never scrolling sideways.
 *
 * A scroller inside the box is held to the same rule: text that makes it
 * scroll sideways is text the user cannot read without a gesture they do not
 * expect. `scrollsSideways` names the scrollers where that is the design.
 *
 * It waits for the frame where the measure holds, because a box opens with a
 * zoom that briefly draws it smaller than it is.
 */
export async function expectContainedInFrame(
  frame: Element,
  { scrollsSideways = [] }: { scrollsSideways?: ReadonlyArray<string> } = {}
) {
  await waitFor(() => {
    const box = frame.getBoundingClientRect()
    expect(box.left).toBeGreaterThanOrEqual(-TOLERANCE)
    expect(box.right).toBeLessThanOrEqual(window.innerWidth + TOLERANCE)

    const escaped = Array.from(frame.querySelectorAll("*"))
      .filter((element) => {
        const rect = element.getBoundingClientRect()
        if (rect.width === 0 && rect.height === 0) return false
        // Visually hidden — Base UI's form inputs, `sr-only` text — is never
        // drawn, wherever it is placed: Base UI pins some at the window's
        // top-left corner.
        const style = getComputedStyle(element)
        if (
          style.visibility === "hidden" ||
          style.clipPath === "inset(50%)" ||
          style.clip === "rect(0px, 0px, 0px, 0px)"
        )
          return false
        if (clippedWithin(element, frame)) return false
        return (
          rect.left < box.left - TOLERANCE || rect.right > box.right + TOLERANCE
        )
      })
      .map(describe)
    expect(escaped).toEqual([])
    expect(frame.scrollWidth).toBeLessThanOrEqual(frame.clientWidth)

    const sideways = Array.from(frame.querySelectorAll("*"))
      .filter((element) => {
        const overflow = getComputedStyle(element).overflowX
        if (overflow !== "auto" && overflow !== "scroll") return false
        if (scrollsSideways.some((selector) => element.matches(selector)))
          return false
        return element.scrollWidth > element.clientWidth
      })
      .map(describe)
    expect(sideways).toEqual([])
  })
}

/**
 * The frame of the open dialog, alert dialog, popover, menu or select — the
 * open one only: a closed popup can stay mounted while it animates out.
 */
export function openFrame(slot: string) {
  return waitFor(() => {
    const frame = document.body.querySelector(
      `[data-slot="${slot}"][data-open]`
    )
    if (!frame) throw new Error(`No open ${slot}`)
    return frame
  })
}
