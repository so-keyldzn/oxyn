/**
 * Where the `@` list goes, from the field's box and the window's.
 *
 * The composer sits at the bottom of the panel: below it there is usually
 * nothing. The list opens **above** the field, and goes below only when above
 * is too short and below is taller. It never overflows the window, nor the
 * field's width — the field is inside the panel, so the panel is never
 * overflowed either.
 */

export interface Box {
  top: number
  bottom: number
  left: number
  width: number
}

export interface Viewport {
  width: number
  height: number
}

export interface ListPlacement {
  side: "top" | "bottom"
  /** Distance from the viewport's left edge. */
  left: number
  width: number
  /** From the viewport's top edge when `side` is `bottom`. */
  top: number | null
  /** From the viewport's bottom edge when `side` is `top`. */
  bottom: number | null
  maxHeight: number
}

/** 18rem: about eight rows, then the list scrolls. */
export const LIST_MAX_HEIGHT = 288
export const LIST_MAX_WIDTH = 288
/** Kept free between the list and the window's edge. */
export const COLLISION_PADDING = 8
/** Between the field and the list. */
const GAP = 4
/** Below this, a side is too short to read a list from. */
const MIN_COMFORTABLE = 120

export function placeList(field: Box, viewport: Viewport): ListPlacement {
  const above = Math.max(0, field.top - GAP - COLLISION_PADDING)
  const below = Math.max(
    0,
    viewport.height - field.bottom - GAP - COLLISION_PADDING
  )
  const side =
    above >= Math.min(LIST_MAX_HEIGHT, MIN_COMFORTABLE) || above >= below
      ? "top"
      : "bottom"
  const room = side === "top" ? above : below

  const width = Math.max(
    0,
    Math.min(
      LIST_MAX_WIDTH,
      field.width,
      viewport.width - 2 * COLLISION_PADDING
    )
  )
  const left = Math.min(
    Math.max(field.left, COLLISION_PADDING),
    viewport.width - COLLISION_PADDING - width
  )

  return {
    side,
    left,
    width,
    top: side === "bottom" ? field.bottom + GAP : null,
    bottom: side === "top" ? viewport.height - field.top + GAP : null,
    maxHeight: Math.min(LIST_MAX_HEIGHT, room),
  }
}
