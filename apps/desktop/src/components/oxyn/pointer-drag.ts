// Dragging inside the window, on pointer events
// (docs/adr/0041-registre-d-actions-menus-et-raccourcis.md, point 9).
//
// Not the HTML5 drag and drop API: files dropped from the system reach Rust
// only while `dragDropEnabled` is on, and on Windows that setting disables
// HTML5 drag and drop in the page. So a drag here is a press, a move past a
// threshold, and a release — and every drag has a keyboard equivalent,
// declared in the action manifest (UX-SPEC « Souris et glisser »).

import * as React from "react"

import { presses } from "@/lib/actions/manifest"

/** Pixels the pointer travels before a press becomes a drag, not a click. */
export const DRAG_THRESHOLD = 4

export interface DragPoint {
  x: number
  y: number
}

interface DragHandlers {
  /** The press became a drag. */
  onStart: (point: DragPoint) => void
  onMove: (point: DragPoint) => void
  onDrop: (point: DragPoint) => void
  /** Esc, a lost pointer, or the window losing focus. */
  onCancel: () => void
}

/**
 * Follows a press from `event` until release. Below the threshold nothing is
 * called and the click happens as usual; past it, the click that ends the
 * drag is swallowed — releasing a dragged catalog row must not also open it.
 * Only the primary button drags: a middle click closes a tab.
 */
export function trackPointerDrag(
  event: React.PointerEvent | PointerEvent,
  handlers: DragHandlers
) {
  if (event.button !== 0 || event.ctrlKey || event.metaKey) return
  const origin = { x: event.clientX, y: event.clientY }
  const pointer = event.pointerId
  let dragging = false

  const pointOf = (moved: PointerEvent) => ({
    x: moved.clientX,
    y: moved.clientY,
  })
  const onPointerMove = (moved: PointerEvent) => {
    if (moved.pointerId !== pointer) return
    const point = pointOf(moved)
    if (!dragging) {
      if (Math.hypot(point.x - origin.x, point.y - origin.y) < DRAG_THRESHOLD)
        return
      dragging = true
      handlers.onStart(point)
    }
    // No text selection follows the pointer while it carries something.
    moved.preventDefault()
    handlers.onMove(point)
  }
  const onPointerUp = (released: PointerEvent) => {
    if (released.pointerId !== pointer) return
    stop()
    if (!dragging) return
    swallowNextClick()
    handlers.onDrop(pointOf(released))
  }
  const cancel = () => {
    stop()
    if (dragging) handlers.onCancel()
  }
  const onKeyDown = (key: KeyboardEvent) => {
    if (key.key !== "Escape" || !dragging) return
    // The drag takes this Esc: it must not also cancel a running query.
    key.preventDefault()
    key.stopPropagation()
    cancel()
  }
  const onPointerCancel = (lost: PointerEvent) => {
    if (lost.pointerId === pointer) cancel()
  }
  function stop() {
    window.removeEventListener("pointermove", onPointerMove)
    window.removeEventListener("pointerup", onPointerUp)
    window.removeEventListener("pointercancel", onPointerCancel)
    window.removeEventListener("keydown", onKeyDown, true)
    window.removeEventListener("blur", cancel)
  }
  window.addEventListener("pointermove", onPointerMove)
  window.addEventListener("pointerup", onPointerUp)
  window.addEventListener("pointercancel", onPointerCancel)
  // Capture, before the action dispatcher reads Esc as `Cancel`.
  window.addEventListener("keydown", onKeyDown, true)
  window.addEventListener("blur", cancel)
}

function swallowNextClick() {
  const swallow = (click: MouseEvent) => {
    click.preventDefault()
    click.stopPropagation()
  }
  window.addEventListener("click", swallow, { capture: true, once: true })
  // A release outside the pressed element fires no click: the listener must
  // not wait for, and eat, the next unrelated one.
  window.setTimeout(
    () => window.removeEventListener("click", swallow, { capture: true }),
    0
  )
}

/**
 * Where an item of a row lands when dropped at `x`, as its index in the
 * reordered row. `centers` are the horizontal centers of the items in their
 * current order; the dragged one does not count. Pure, so it is tested.
 */
export function dropIndex(
  centers: ReadonlyArray<number>,
  from: number,
  x: number
) {
  let index = 0
  for (const [position, center] of centers.entries()) {
    if (position !== from && center < x) index++
  }
  return index
}

/** `items` with the one at `from` moved to `to`. Pure; out of range is a copy. */
export function moveItem<T>(items: ReadonlyArray<T>, from: number, to: number) {
  const moved = [...items]
  if (from < 0 || from >= items.length || to < 0 || to >= items.length)
    return moved
  const [item] = moved.splice(from, 1)
  moved.splice(to, 0, item as T)
  return moved
}

/**
 * `items` in the order the user left them: those named in `order` first, as
 * ordered, then the others — opened since — in their own order. A key of
 * `order` no longer among `items` is ignored. Pure, so it is tested.
 */
export function inOrder<T>(
  items: ReadonlyArray<T>,
  keyOf: (item: T) => string,
  order: ReadonlyArray<string>
) {
  const rank = new Map(order.map((key, index) => [key, index]))
  const placed = items
    .filter((item) => rank.has(keyOf(item)))
    .sort((a, b) => (rank.get(keyOf(a)) ?? 0) - (rank.get(keyOf(b)) ?? 0))
  return [...placed, ...items.filter((item) => !rank.has(keyOf(item)))]
}

/**
 * The insertion mark an item draws while another is dragged over the row:
 * the dragged item lands before the item at `to` when moving left, after it
 * when moving right.
 */
export function dropMark(
  drag: { from: number; to: number } | null,
  index: number
): "before" | "after" | null {
  if (!drag || drag.from === drag.to || index !== drag.to) return null
  return drag.to < drag.from ? "before" : "after"
}

/**
 * Reordering a row of items by dragging one of them. `centers` is read when
 * the drag starts and on each move, so a row that scrolls meanwhile is read
 * where it is.
 */
export function usePointerReorder({
  centers,
  onMove,
}: {
  centers: () => ReadonlyArray<number>
  onMove: (from: number, to: number) => void
}) {
  const [drag, setDrag] = React.useState<{ from: number; to: number } | null>(
    null
  )
  const latest = React.useRef({ centers, onMove })
  latest.current = { centers, onMove }
  const start = (event: React.PointerEvent, from: number) => {
    let to = from
    const follow = (point: DragPoint) => {
      to = dropIndex(latest.current.centers(), from, point.x)
      setDrag((current) =>
        current?.from === from && current.to === to ? current : { from, to }
      )
    }
    trackPointerDrag(event, {
      onStart: follow,
      onMove: follow,
      onDrop: (point) => {
        follow(point)
        setDrag(null)
        if (to !== from) latest.current.onMove(from, to)
      },
      onCancel: () => setDrag(null),
    })
  }
  return { drag, start }
}

/** The keyboard's move of a tab or a column, as the manifest binds it (⌥⇧←, ⌥⇧→). */
export function moveStep(event: React.KeyboardEvent | KeyboardEvent) {
  if (presses("item.moveLeft", event)) return -1
  if (presses("item.moveRight", event)) return 1
  return 0
}
