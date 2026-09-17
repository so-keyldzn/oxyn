import { describe, expect, it } from "vitest"

import { addressKey, staleExpanded } from "./catalog-tree"
import { freshnessLabel } from "./facet-frame"
import { sizeLabel } from "./object-inspector"
import { draftDiffers, previewNotice } from "./preview-controls"
import { findSummary } from "./result-find-bar"
import { renderedBytes } from "./value-page-dialog"
import { PLAIN_SHAPE } from "@/lib/ipc/metadata"
import type { CatalogNode } from "@/lib/ipc/types"

describe("previewNotice", () => {
  const loaded = {
    status: "loaded" as const,
    pending: false,
    filtered: false,
    applied: PLAIN_SHAPE,
  }

  it("warns on a refused read, whatever the pagination says", () => {
    const notice = previewNotice({
      ...loaded,
      status: "failed",
      pagination: { type: "ready", previous: true, next: true, firstRow: 201 },
    })
    expect(notice?.warning).toBe(true)
    expect(notice?.text).toMatch(/refused/)
  })

  it("tells an empty filter from an empty table", () => {
    const base = { ...loaded, status: "empty" as const, pagination: null }
    expect(previewNotice({ ...base, filtered: true })?.text).toMatch(
      /No row matches this filter/
    )
    expect(previewNotice(base)?.text).toBe("This table holds no rows.")
  })

  it("says why there is no page without a total order", () => {
    expect(
      previewNotice({ ...loaded, pagination: { type: "needsOrder" } })?.text
    ).toMatch(/no guaranteed order/)
    expect(
      previewNotice({ ...loaded, pagination: { type: "noUniqueKey" } })?.text
    ).toMatch(/no unique key/)
  })

  it("says a page after the first is a new read", () => {
    const text = previewNotice({
      ...loaded,
      applied: {
        ...PLAIN_SHAPE,
        sort: [{ column: "id", descending: false }],
        offset: 1200,
      },
      pagination: {
        type: "ready",
        previous: true,
        next: false,
        firstRow: 1201,
      },
    })?.text
    expect(text).toMatch(/^Rows 1,201–1,400 of this order/)
  })

  it("stays silent on a first ordered page", () => {
    expect(
      previewNotice({
        ...loaded,
        pagination: { type: "ready", previous: false, next: true, firstRow: 1 },
      })
    ).toBeNull()
  })

  it("puts a typed but unapplied filter before the pagination", () => {
    expect(
      previewNotice({
        ...loaded,
        pending: true,
        pagination: { type: "needsOrder" },
      })?.text
    ).toMatch(/not applied yet/)
  })

  it("compares drafts without surrounding spaces", () => {
    expect(draftDiffers("  id = 1 ", "id = 1")).toBe(false)
    expect(draftDiffers("", null)).toBe(false)
    expect(draftDiffers("id = 2", "id = 1")).toBe(true)
  })
})

describe("findSummary", () => {
  const answer = {
    total: 0,
    row: null,
    ordinal: null,
    skippedBatches: 0,
    capped: false,
  }

  it("says no match only when everything was searched", () => {
    expect(findSummary(answer)).toEqual({ text: "No match", partial: false })
    const partial = findSummary({ ...answer, skippedBatches: 1 })
    expect(partial.partial).toBe(true)
    expect(partial.text).toBe(
      "No match · 1 batch not searched: they spilled to disk"
    )
  })

  it("places the current match among the others", () => {
    expect(
      findSummary({ ...answer, total: 1234, row: 9, ordinal: 3 }).text
    ).toBe("3 of 1,234 matching rows")
    expect(findSummary({ ...answer, total: 1, row: 0, ordinal: 1 }).text).toBe(
      "1 of 1 matching row"
    )
  })

  it("says a capped count is a lower bound", () => {
    expect(
      findSummary({
        ...answer,
        total: 50_000,
        row: 0,
        ordinal: 1,
        capped: true,
      }).text
    ).toBe("1 of 50,000 matching rows or more · the search stopped counting")
  })
})

describe("freshnessLabel", () => {
  it("names each state", () => {
    expect(freshnessLabel({ state: "never" })).toBe("Not loaded")
    expect(freshnessLabel({ state: "invalidated" })).toBe("Stale")
    expect(
      freshnessLabel({
        state: "fetched",
        fetchedAt: "2026-09-15T09:30:00+00:00",
      })
    ).toMatch(/^Loaded \d\d:\d\d/)
  })

  it("does not invent a time it cannot read", () => {
    expect(freshnessLabel({ state: "fetched", fetchedAt: "not a date" })).toBe(
      "Loaded"
    )
  })
})

describe("renderedBytes", () => {
  const page = {
    column: "notes",
    dataType: "Utf8",
    isNull: false,
    text: "",
    offset: 16_380,
    nextOffset: 32_760,
    totalBytes: 48_213,
  }

  it("bounds a middle page by the next offset", () => {
    expect(renderedBytes(page)).toBe("Rendered bytes 16,380–32,760 of 48,213")
  })

  it("bounds the last page by the total", () => {
    expect(renderedBytes({ ...page, offset: 32_760, nextOffset: null })).toBe(
      "Rendered bytes 32,760–48,213 of 48,213"
    )
  })
})

describe("sizeLabel", () => {
  it("keeps bytes whole and rounds larger units", () => {
    expect(sizeLabel(512)).toBe("512 B")
    expect(sizeLabel(412_000_000)).toBe("392.9 MiB")
  })
})

describe("staleExpanded", () => {
  const node = (
    relation: string | null,
    namespace: string,
    stale: boolean,
    children: Array<CatalogNode> = []
  ): CatalogNode => ({
    address: { catalog: null, namespace, relation },
    name: relation ?? namespace,
    kind: relation ? "table" : "namespace",
    holdsRecords: relation !== null,
    system: false,
    comment: null,
    loaded: true,
    stale,
    children,
  })

  it("lists only the stale levels the user has open", () => {
    const freshChild = node(null, "public.inner", false)
    const staleChild = node(null, "public.archive", true)
    const open = node(null, "public", true, [freshChild, staleChild])
    const closed = node(null, "reporting", true)
    const expanded = new Set(
      [open, freshChild, staleChild].map((level) => addressKey(level.address))
    )
    expect(staleExpanded([open, closed], expanded)).toEqual([
      open.address,
      staleChild.address,
    ])
  })

  it("does not look inside a closed level", () => {
    const hidden = node(null, "public.archive", true)
    const closed = node(null, "public", false, [hidden])
    expect(
      staleExpanded([closed], new Set([addressKey(hidden.address)]))
    ).toEqual([])
  })
})
