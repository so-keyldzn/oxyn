import { describe, expect, it, vi } from "vitest"

import { KEPT_PREVIEWS, createPreviewStore } from "./preview-store"
import type { PreviewEffects } from "./preview-store"
import { PLAIN_SHAPE } from "@/lib/ipc/metadata"
import type { CommandOutcome } from "@/lib/ipc/types"

const target = {
  connection: "c1",
  session: "s1",
  address: { catalog: null, namespace: "public", relation: "invoices" },
  serverCancel: true,
}

const executed = (result: string, rows = 3): CommandOutcome => ({
  type: "executed",
  result,
  columns: [{ name: "id", dataType: "Int64", nullable: false }],
  rows,
  elapsedMs: 12,
  complete: true,
  cancelled: false,
  truncated: false,
})

/** A backend whose answers the test releases by hand. */
function harness() {
  const pending = new Map<
    string,
    {
      resolve: (outcome: CommandOutcome) => void
      reject: (error: Error) => void
    }
  >()
  let next = 0
  const sessions: Array<string> = []
  const effects: PreviewEffects = {
    read: (id, entry) => {
      sessions.push(entry.session)
      return new Promise((resolve, reject) =>
        pending.set(id, { resolve, reject })
      )
    },
    cancel: vi.fn(),
    refuse: vi.fn(),
    forgetResult: vi.fn(),
    watch: vi.fn(),
    unwatch: vi.fn(),
    newId: () => `cmd${++next}`,
    now: () => next,
  }
  const previews = createPreviewStore(effects)
  const answer = async (id: string, outcome: CommandOutcome) => {
    pending.get(id)?.resolve(outcome)
    await Promise.resolve()
    await Promise.resolve()
  }
  return { previews, effects, answer, pending, sessions }
}

describe("preview store", () => {
  it("keeps a read preview for the next view instead of reading again", async () => {
    const { previews, effects, answer } = harness()
    previews.attach("k", target)
    previews.read("k", PLAIN_SHAPE)
    await answer("cmd1", executed("r1"))
    previews.detach("k")
    expect(effects.cancel).not.toHaveBeenCalled()
    expect(effects.forgetResult).not.toHaveBeenCalled()

    previews.attach("k", target)
    const entry = previews.store.state.k
    expect(entry?.state).toMatchObject({
      status: "populated",
      result: "r1",
      elapsedMs: 12,
    })
    expect(entry?.views).toBe(1)
  })

  it("never releases the shown result on a re-render or a tab change", async () => {
    const { previews, effects, answer } = harness()
    // Two views of the same object — a re-render, a second tab — then one
    // of them goes away.
    previews.attach("k", target)
    previews.read("k", PLAIN_SHAPE)
    await answer("cmd1", executed("r1"))
    previews.attach("k", target)
    previews.detach("k")
    expect(effects.forgetResult).not.toHaveBeenCalled()
    expect(effects.cancel).not.toHaveBeenCalled()
    expect(previews.store.state.k?.state).toMatchObject({ result: "r1" })

    // The last view leaves: the rows are kept for the next visit.
    previews.detach("k")
    expect(effects.forgetResult).not.toHaveBeenCalled()
    expect(previews.store.state.k?.state).toMatchObject({ result: "r1" })
  })

  it("keeps the shown result across a StrictMode double mount", async () => {
    const { previews, effects, answer } = harness()
    previews.attach("k", target)
    previews.read("k", PLAIN_SHAPE)
    await answer("cmd1", executed("r1"))
    // React mounts, unmounts and mounts again in development.
    previews.detach("k")
    previews.attach("k", target)
    expect(effects.forgetResult).not.toHaveBeenCalled()
    expect(previews.store.state.k?.state).toMatchObject({ result: "r1" })
    expect(previews.store.state.k?.views).toBe(1)
  })

  it("cancels a read nobody shows any more, and releases its late answer", async () => {
    const { previews, effects, answer } = harness()
    previews.attach("k", target)
    previews.read("k", PLAIN_SHAPE)
    previews.detach("k")
    expect(effects.cancel).toHaveBeenCalledWith("cmd1")
    expect(previews.store.state.k).toBeUndefined()

    await answer("cmd1", executed("late"))
    expect(effects.forgetResult).toHaveBeenCalledWith("late")
  })

  it("never shows the answer of a replaced read", async () => {
    const { previews, effects, answer } = harness()
    previews.attach("k", target)
    previews.read("k", PLAIN_SHAPE)
    const sorted = {
      ...PLAIN_SHAPE,
      sort: [{ column: "id", descending: true }],
    }
    previews.read("k", sorted)
    expect(effects.cancel).toHaveBeenCalledWith("cmd1")

    await answer("cmd1", executed("old"))
    expect(effects.forgetResult).toHaveBeenCalledWith("old")
    expect(previews.store.state.k?.state.status).toBe("running")

    await answer("cmd2", executed("new"))
    expect(previews.store.state.k?.state).toMatchObject({ result: "new" })
    expect(previews.store.state.k?.applied).toEqual(sorted)
  })

  it("keeps the shape in force when a read is refused", async () => {
    const { previews, answer } = harness()
    previews.attach("k", target)
    previews.read("k", { ...PLAIN_SHAPE, predicate: "id >< 3" })
    await answer("cmd1", { type: "denied", reason: "syntax error" })
    const entry = previews.store.state.k
    expect(entry?.state).toMatchObject({
      status: "error",
      message: "syntax error",
    })
    expect(entry?.applied).toEqual(PLAIN_SHAPE)
  })

  it("refuses an approval instead of waiting for one", async () => {
    const { previews, effects, answer } = harness()
    previews.attach("k", target)
    previews.read("k", PLAIN_SHAPE)
    await answer("cmd1", {
      type: "needsApproval",
      command: "cmd1",
      reason: "policy",
      preview: null,
    })
    expect(effects.refuse).toHaveBeenCalledWith("cmd1")
    expect(previews.store.state.k?.approvalRefused).toBe(true)
  })

  it("sends a cancel once, however often Cancel is pressed", () => {
    const { previews, effects } = harness()
    previews.attach("k", target)
    previews.read("k", PLAIN_SHAPE)
    previews.cancel("k")
    previews.cancel("k")
    expect(effects.cancel).toHaveBeenCalledTimes(1)
    expect(previews.store.state.k?.cancelling).toBe(true)
  })

  it("releases the previous result when reading again", async () => {
    const { previews, effects, answer } = harness()
    previews.attach("k", target)
    previews.read("k", PLAIN_SHAPE)
    await answer("cmd1", executed("r1"))
    previews.read("k", PLAIN_SHAPE)
    expect(effects.forgetResult).toHaveBeenCalledWith("r1")
  })

  it("evicts the oldest unseen previews beyond the limit", async () => {
    const { previews, effects, answer } = harness()
    for (let index = 0; index <= KEPT_PREVIEWS; index++) {
      const key = `k${index}`
      previews.attach(key, target)
      previews.read(key, PLAIN_SHAPE)
      await answer(`cmd${index + 1}`, executed(`r${index}`))
      previews.detach(key)
    }
    expect(Object.keys(previews.store.state)).toHaveLength(KEPT_PREVIEWS)
    expect(previews.store.state.k0).toBeUndefined()
    expect(effects.forgetResult).toHaveBeenCalledWith("r0")
  })

  it("owes one more read when a change arrives during a read", async () => {
    const { previews, answer } = harness()
    previews.attach("k", target)
    previews.read("k", PLAIN_SHAPE)
    previews.invalidate("c1")
    await answer("cmd1", executed("r1"))
    expect(previews.store.state.k?.stale).toBe(true)
    previews.read("k", PLAIN_SHAPE)
    expect(previews.store.state.k?.stale).toBe(false)
  })

  it("marks only the previews of the changed connection stale", () => {
    const { previews } = harness()
    previews.attach("a", target)
    previews.attach("b", { ...target, connection: "c2" })
    previews.invalidate("c1")
    expect(previews.store.state.a?.stale).toBe(true)
    expect(previews.store.state.b?.stale).toBe(false)
  })

  it("reports a failed read with whether it is retryable", async () => {
    const { previews, pending } = harness()
    previews.attach("k", target)
    previews.read("k", PLAIN_SHAPE)
    pending
      .get("cmd1")
      ?.reject(
        Object.assign(new Error("connection reset"), { retryable: true })
      )
    await Promise.resolve()
    await Promise.resolve()
    expect(previews.store.state.k?.state).toEqual({
      status: "error",
      message: "connection reset",
      retryable: true,
    })
  })

  describe("after a reconnection", () => {
    const reopened = { ...target, session: "s2" }

    it("reads on the new session only, keeping the shape in force", async () => {
      const { previews, effects, answer, sessions } = harness()
      const sorted = {
        ...PLAIN_SHAPE,
        sort: [{ column: "id", descending: true }],
      }
      previews.attach("k", target)
      previews.read("k", sorted)
      await answer("cmd1", executed("r1"))
      previews.detach("k")

      previews.attach("k", reopened)
      const entry = previews.store.state.k
      // The rows came from the closed session: released, not shown as current.
      expect(effects.forgetResult).toHaveBeenCalledWith("r1")
      expect(entry?.state.status).toBe("initial")
      expect(entry?.session).toBe("s2")
      expect(entry?.applied).toEqual(sorted)
      expect(entry?.views).toBe(1)

      previews.read("k", entry?.applied ?? PLAIN_SHAPE)
      previews.read("k", entry?.applied ?? PLAIN_SHAPE)
      expect(sessions).toEqual(["s1", "s2", "s2"])
    })

    it("never lets a late answer from the old session replace the new one", async () => {
      const { previews, effects, answer } = harness()
      previews.attach("k", target)
      previews.read("k", PLAIN_SHAPE)
      previews.attach("k", reopened)
      expect(effects.cancel).toHaveBeenCalledWith("cmd1")
      previews.read("k", PLAIN_SHAPE)

      await answer("cmd1", executed("old"))
      expect(effects.forgetResult).toHaveBeenCalledWith("old")
      expect(previews.store.state.k?.state.status).toBe("running")

      await answer("cmd2", executed("new"))
      expect(previews.store.state.k?.state).toMatchObject({ result: "new" })
    })

    it("keeps an entry attached again on the same session", async () => {
      const { previews, effects, answer } = harness()
      previews.attach("k", target)
      previews.read("k", PLAIN_SHAPE)
      await answer("cmd1", executed("r1"))
      previews.detach("k")
      previews.attach("k", { ...target })
      expect(effects.forgetResult).not.toHaveBeenCalled()
      expect(previews.store.state.k?.state).toMatchObject({ result: "r1" })
    })

    it("drops the previews of closed sessions, and cancels their reads", async () => {
      const { previews, effects, answer } = harness()
      previews.attach("kept", target)
      previews.read("kept", PLAIN_SHAPE)
      await answer("cmd1", executed("r1"))
      previews.detach("kept")
      previews.attach("shown", target)
      previews.read("shown", PLAIN_SHAPE)
      previews.attach("other", { ...target, connection: "c2", session: "t1" })

      previews.closeSessions(["s1"])
      expect(effects.forgetResult).toHaveBeenCalledWith("r1")
      expect(effects.cancel).toHaveBeenCalledWith("cmd2")
      expect(Object.keys(previews.store.state)).toEqual(["other"])

      await answer("cmd2", executed("late"))
      expect(effects.forgetResult).toHaveBeenCalledWith("late")
      expect(previews.store.state.shown).toBeUndefined()
    })
  })
})
