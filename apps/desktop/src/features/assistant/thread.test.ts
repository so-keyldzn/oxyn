import { describe, expect, it } from "vitest"

import {
  NEW_THREAD,
  activePath,
  addNode,
  applyUpdate,
  select,
  threadOf,
  versionsOf,
} from "./thread"

const finished = {
  kind: "finished",
  ending: { type: "answered", turns: 1, truncated: false, cut: null },
} as const

describe("a conversation tree", () => {
  it("shows the newest version under each parent, and what followed it", () => {
    let thread = addNode(NEW_THREAD, 0, null, "first")
    thread = applyUpdate(thread, { node: 0, event: finished })
    thread = addNode(thread, 1, 0, "follow-up")
    thread = addNode(thread, 2, null, "first, edited")
    expect(activePath(thread).map((node) => node.id)).toEqual([2])

    const edited = activePath(thread)[0]
    expect(edited && versionsOf(thread, edited)).toEqual({
      position: 2,
      count: 2,
      previous: 0,
      next: null,
    })

    thread = select(thread, 0)
    expect(activePath(thread).map((node) => node.id)).toEqual([0, 1])
  })

  it("ends a run on its node only", () => {
    let thread = addNode(NEW_THREAD, 0, null, "q")
    expect(thread.running).toBe(0)
    thread = applyUpdate(thread, {
      node: 0,
      event: { kind: "textDelta", text: "a" },
    })
    expect(thread.running).toBe(0)
    thread = applyUpdate(thread, { node: 0, event: finished })
    expect(thread.running).toBeNull()
    expect(thread.nodes[0]?.exchange.running).toBe(false)
  })

  it("ignores an event for a node it does not know yet", () => {
    expect(applyUpdate(NEW_THREAD, { node: 4, event: finished })).toBe(
      NEW_THREAD
    )
  })

  it("takes a conversation back without claiming a run the backend dropped", () => {
    const thread = threadOf({
      id: "7",
      title: "Clients",
      nodes: [
        {
          id: 0,
          parent: null,
          mentions: [],
          question: "How many?",
          events: [
            { kind: "question", text: "How many?", mentions: [] },
            { kind: "textDelta", text: "Some" },
          ],
        },
      ],
      selections: [{ parent: null, node: 0 }],
      running: null,
    })
    expect(thread.nodes[0]?.exchange.running).toBe(false)
    expect(activePath(thread)).toHaveLength(1)
  })
})
