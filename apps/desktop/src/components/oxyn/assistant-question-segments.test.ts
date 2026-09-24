import { describe, expect, it } from "vitest"

import { questionSegments } from "./assistant-question-segments"
import type { MentionView } from "@/lib/ipc/ai"

function table(label: string): MentionView {
  return {
    kind: "table",
    label,
    mention: {
      kind: "relation",
      address: { catalog: null, namespace: "public", relation: label },
      field: null,
    },
    missing: false,
  }
}

const shape = (text: string, mentions: Array<MentionView>) => {
  const { segments, unplaced } = questionSegments(text, mentions)
  return {
    segments: segments.map((segment) =>
      segment.kind === "text" ? segment.text : `[${segment.view.label}]`
    ),
    unplaced: unplaced.map((view) => view.label),
  }
}

describe("a question's chips", () => {
  it("sit where the mentions were typed, in order", () => {
    expect(
      shape("join @orders with @customers on id", [
        table("orders"),
        table("customers"),
      ])
    ).toEqual({
      segments: ["join ", "[orders]", " with ", "[customers]", " on id"],
      unplaced: [],
    })
  })

  it("never guess: an @word the question does not carry stays text", () => {
    expect(shape("mail me @home about @orders", [table("orders")])).toEqual({
      segments: ["mail me @home about ", "[orders]"],
      unplaced: [],
    })
  })

  it("skip a longer word that starts like the label", () => {
    expect(shape("@orders_v then @orders", [table("orders")])).toEqual({
      segments: ["@orders_v then ", "[orders]"],
      unplaced: [],
    })
  })

  it("keep a mention whose label is not in the text, apart", () => {
    expect(shape("count them", [table("orders")])).toEqual({
      segments: ["count them"],
      unplaced: ["orders"],
    })
  })
})
