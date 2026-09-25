import { describe, expect, it } from "vitest"

import {
  Cell,
  ConnectionDraft,
  ConnectionTest,
  ExecutionEvent,
  RelationDetail,
  ResultWindow,
} from "./types"

// These schemas are the mirror of `crates/oxyn-desktop/src/ipc.rs`, and what
// they protect is the boundary's behaviour on shapes the Rust side really
// produces — flattened variants, present-but-null fields, tags that are told
// apart by value. ADR-0031.

describe("cells", () => {
  it("accepts the four shapes the grid knows how to draw", () => {
    for (const value of [
      null,
      "",
      "text",
      { text: "truncated", fullBytes: 4096 },
      { unrenderable: "bytea" },
    ]) {
      expect(Cell.safeParse(value).success).toBe(true)
    }
  })

  it("rejects what the grid could not draw, rather than letting it through", () => {
    // The cheap positional check still has to be a check: a number reaching a
    // cell renderer is the kind of thing that used to surface as `undefined`.
    for (const value of [1, true, [], {}, { other: "key" }]) {
      expect(Cell.safeParse(value).success).toBe(false)
    }
  })

  it("checks the payload, not just the presence of the key", () => {
    // `{ text: <object> }` reaches `cell.text.length + 8` in the grid and
    // renders `NaN` — a broken cell instead of the named error the rest of
    // the boundary produces.
    for (const value of [
      { text: {}, fullBytes: 1 },
      { text: 42, fullBytes: 1 },
      { unrenderable: 42 },
    ]) {
      expect(Cell.safeParse(value).success).toBe(false)
    }
  })
})

describe("result windows", () => {
  const page = {
    type: "page",
    offset: 0,
    rows: [["a", null]],
    totalRows: 1,
    complete: true,
  }

  it("carries the page's own fields beside the tag, as serde flattens them", () => {
    const parsed = ResultWindow.parse(page)
    expect(parsed).toMatchObject({ type: "page", offset: 0, totalRows: 1 })
  })

  it("tells a page from an expired retention by the tag's value, not its presence", () => {
    expect(ResultWindow.parse({ type: "expired" })).toEqual({
      type: "expired",
    })
    // A page whose tag went missing is not an `expired`: it is unreadable.
    const { type: _dropped, ...untagged } = page
    expect(ResultWindow.safeParse(untagged).success).toBe(false)
  })

  it("refuses a negative offset, which no page has", () => {
    expect(ResultWindow.safeParse({ ...page, offset: -1 }).success).toBe(false)
  })
})

describe("execution events", () => {
  it("reads `command` at the same level as the variant's own fields", () => {
    const parsed = ExecutionEvent.parse({
      command: "c1",
      type: "completed",
      result: "r1",
      rows: 150,
      elapsedMs: 12,
    })
    expect(parsed).toMatchObject({ command: "c1", result: "r1", rows: 150 })
  })

  it("refuses an event with no command to attribute it to", () => {
    const orphan = { type: "cancelled" }
    expect(ExecutionEvent.safeParse(orphan).success).toBe(false)
  })

  it("reads a transaction state and refuses one it does not know", () => {
    // `TransactionStateView` in ipc.rs: camelCase unit variants.
    const parsed = ExecutionEvent.parse({
      command: "c1",
      type: "transactionState",
      session: "s1",
      state: "open",
    })
    expect(parsed).toMatchObject({ session: "s1", state: "open" })
    expect(
      ExecutionEvent.safeParse({ ...parsed, state: "aborted" }).success
    ).toBe(false)
  })
})

describe("optional fields", () => {
  it("expects `None` as a present null, which is how serde writes it", () => {
    // No `skip_serializing_if` exists on the Rust side, so the key is always
    // there. A schema written with `.optional()` would accept a missing key
    // and hide a renamed field — the defect this boundary exists to catch.
    const detail = {
      name: "users",
      kind: "table",
      comment: null,
      estimatedRows: null,
      sizeBytes: null,
      fields: [],
    }
    expect(RelationDetail.safeParse(detail).success).toBe(true)

    const { comment: _absent, ...missing } = detail
    expect(RelationDetail.safeParse(missing).success).toBe(false)
  })
})

describe("connection drafts", () => {
  const draft = {
    driver: "postgres",
    name: "analytics",
    environment: "production",
    privacyTier: "metadata",
    readOnly: false,
    values: {},
    secrets: {},
  }

  it("requires the markings `ipc.rs` refuses to default", () => {
    // Both were missing from this mirror while the form sent them anyway.
    // Defaulting either here would hide a front that forgot the user's
    // choice — I-02 for the environment, I-04 for the tier.
    expect(ConnectionDraft.safeParse(draft).success).toBe(true)
    for (const key of ["environment", "privacyTier", "readOnly"] as const) {
      const { [key]: _dropped, ...incomplete } = draft
      expect(ConnectionDraft.safeParse(incomplete).success).toBe(false)
    }
  })

  it("refuses an environment it does not know, rather than treating it as local", () => {
    expect(
      ConnectionDraft.safeParse({ ...draft, environment: "prod" }).success
    ).toBe(false)
  })
})

describe("connection tests", () => {
  it("reads the three answers `ConnectionTest` serializes to", () => {
    for (const value of [
      { type: "succeeded", elapsedMs: 42 },
      {
        type: "failed",
        message: "connection failed: connection refused",
        class: "transient",
        retryable: true,
      },
      { type: "cancelled" },
    ]) {
      expect(ConnectionTest.safeParse(value).success).toBe(true)
    }
  })

  it("never reads a success out of a failure missing its class", () => {
    expect(
      ConnectionTest.safeParse({
        type: "failed",
        message: "refused",
        retryable: false,
      }).success
    ).toBe(false)
  })
})
