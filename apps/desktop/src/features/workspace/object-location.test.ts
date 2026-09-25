import { describe, expect, it } from "vitest"

import { OBJECT_TABS } from "@/components/oxyn/object-view-frame"
import { sectionOf, tabOf } from "@/features/workspace/object-location"
import { SectionChoice } from "@/lib/ipc/location"

describe("object tab places", () => {
  it("names every sub-view the view shows, both relation directions apart", () => {
    for (const { value } of OBJECT_TABS) {
      for (const direction of ["incoming", "outgoing"] as const) {
        const section = sectionOf(value, direction)
        expect(SectionChoice.options).toContain(section)
        const back = tabOf(section)
        expect(back.tab).toBe(value)
        if (value === "relations") expect(back.direction).toBe(direction)
      }
    }
  })

  it("opens every saved section on a tab the view has", () => {
    const tabs = OBJECT_TABS.map((tab) => tab.value)
    for (const section of SectionChoice.options)
      expect(tabs).toContain(tabOf(section).tab)
  })
})
