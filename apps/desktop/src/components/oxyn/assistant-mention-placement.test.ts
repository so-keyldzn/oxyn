import { describe, expect, it } from "vitest"

import {
  COLLISION_PADDING,
  LIST_MAX_HEIGHT,
  placeList,
} from "./assistant-mention-placement"

const window800 = { width: 1280, height: 800 }

describe("the @ list's place", () => {
  it("opens above a field at the bottom of the panel", () => {
    const place = placeList(
      { top: 700, bottom: 780, left: 900, width: 360 },
      window800
    )
    expect(place.side).toBe("top")
    expect(place.bottom).toBe(800 - 700 + 4)
    expect(place.maxHeight).toBe(LIST_MAX_HEIGHT)
    expect(place.width).toBe(288)
  })

  it("goes below only when above is too short and below is taller", () => {
    const place = placeList(
      { top: 60, bottom: 140, left: 0, width: 400 },
      window800
    )
    expect(place.side).toBe("bottom")
    expect(place.top).toBe(144)
    expect(place.left).toBe(COLLISION_PADDING)
  })

  it("never runs past a low window", () => {
    const low = { width: 1280, height: 260 }
    const place = placeList(
      { top: 180, bottom: 250, left: 20, width: 300 },
      low
    )
    expect(place.side).toBe("top")
    // What the list can take above the field, padding kept.
    expect(place.maxHeight).toBe(180 - 4 - COLLISION_PADDING)
    expect(place.maxHeight).toBeLessThan(LIST_MAX_HEIGHT)
  })

  it("stays within a 360 px window and within the field", () => {
    const narrow = { width: 360, height: 640 }
    const field = { top: 540, bottom: 620, left: 8, width: 344 }
    const place = placeList(field, narrow)
    expect(place.left).toBeGreaterThanOrEqual(COLLISION_PADDING)
    expect(place.left + place.width).toBeLessThanOrEqual(
      narrow.width - COLLISION_PADDING
    )
    expect(place.width).toBeLessThanOrEqual(field.width)
  })
})
