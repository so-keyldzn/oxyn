import { act, renderHook } from "@testing-library/react"
import { describe, expect, it, vi } from "vitest"

import { useCloseTabs } from "./use-close-tabs"
import type { TabCloseTarget } from "./use-close-tabs"

const tabs: Array<TabCloseTarget> = ["first", "second"].map((key) => ({
  key,
  title: `${key}.sql`,
  blocked: null,
  reasons: {
    title: `${key}.sql`,
    connection: "billing",
    unsaved: true,
    conflict: false,
    running: false,
    transaction: null,
    hasSavedCopy: false,
  },
}))

function setup(targets = tabs, keys = targets.map((tab) => tab.key)) {
  const save = vi.fn((_key: string) => Promise.resolve(true))
  const cancelWrite = vi.fn()
  const close = vi.fn(
    (
      closingKeys: ReadonlyArray<string>,
      _discard: boolean,
      _signal: AbortSignal
    ) => Promise.resolve(closingKeys)
  )
  const hook = renderHook(() =>
    useCloseTabs({ targets: () => targets, save, cancelWrite, close })
  )
  act(() => hook.result.current.request(keys))
  return { hook, save, close, cancelWrite }
}

describe("Close all tabs", () => {
  it("keeps tabs outside the snapshot for Close others or Close to the right", async () => {
    const { hook, close } = setup(tabs, ["second"])
    expect(hook.result.current.review?.tabs.map((tab) => tab.key)).toEqual([
      "second",
    ])
    await act(() => hook.result.current.decide("discard"))
    expect(close.mock.calls[0]?.[0]).toEqual(["second"])
  })
  it("asks once for all unsaved tabs and discards the confirmed snapshot together", async () => {
    const { hook, save, close } = setup()
    expect(hook.result.current.review?.tabs).toEqual(tabs)
    expect(close).not.toHaveBeenCalled()
    await act(() => hook.result.current.decide("discard"))
    expect(close).toHaveBeenCalledExactlyOnceWith(
      ["first", "second"],
      true,
      expect.any(AbortSignal)
    )
    expect(save).not.toHaveBeenCalled()
    expect(hook.result.current.review).toBeNull()
  })

  it("cancels the collective review without closing or saving anything", () => {
    const { hook, close, save } = setup()
    act(() => hook.result.current.cancel())
    expect(close).not.toHaveBeenCalled()
    expect(save).not.toHaveBeenCalled()
    expect(hook.result.current.review).toBeNull()
  })

  it("starts every save together and closes nothing when any save fails", async () => {
    const { hook, save, close } = setup()
    const answers: Array<(saved: boolean) => void> = []
    save.mockImplementation(
      () => new Promise((resolve) => answers.push(resolve))
    )
    let done = Promise.resolve()
    act(() => {
      done = hook.result.current.decide("save")
    })
    expect(save).toHaveBeenCalledTimes(2)
    expect(close).not.toHaveBeenCalled()
    await act(async () => {
      answers[0]?.(true)
      answers[1]?.(false)
      await done
    })
    expect(close).not.toHaveBeenCalled()
    expect(hook.result.current.review?.notice).toContain("second.sql")
    expect(hook.result.current.review?.tabs).toHaveLength(2)
  })

  it("waits for all successful saves before sending one batch close", async () => {
    const { hook, save, close } = setup()
    await act(() => hook.result.current.decide("save"))
    expect(save).toHaveBeenCalledTimes(2)
    expect(close).toHaveBeenCalledExactlyOnceWith(
      ["first", "second"],
      false,
      expect.any(AbortSignal)
    )
    expect(hook.result.current.review).toBeNull()
  })

  it("cancels pending saves and ignores their late answers", async () => {
    const { hook, save, close, cancelWrite } = setup()
    const answers: Array<(saved: boolean) => void> = []
    save.mockImplementation(
      () => new Promise((resolve) => answers.push(resolve))
    )
    let done = Promise.resolve()
    act(() => {
      done = hook.result.current.decide("save")
    })
    act(() => hook.result.current.cancel())
    expect(cancelWrite.mock.calls).toEqual([["first"], ["second"]])
    await act(async () => {
      answers.forEach((answer) => answer(true))
      await done
    })
    expect(close).not.toHaveBeenCalled()
    expect(hook.result.current.review).toBeNull()
  })

  it("keeps failed closes in one review and never closes a newly opened tab", async () => {
    const targets = [...tabs]
    const { hook, close } = setup(targets)
    targets.push({ key: "new", title: "New", reasons: null, blocked: null })
    close.mockResolvedValue(["first"])
    await act(() => hook.result.current.decide("discard"))
    expect(close.mock.calls[0]?.[0]).toEqual(["first", "second"])
    expect(hook.result.current.review?.tabs.map((tab) => tab.key)).toEqual([
      "second",
    ])
    expect(hook.result.current.review?.notice).toContain("could not be closed")
  })

  it("closes clean tabs without a confirmation and keeps blocked exports open", async () => {
    const clean = setup([
      { key: "table", title: "Table", reasons: null, blocked: null },
    ])
    await act(async () => {})
    expect(clean.close).toHaveBeenCalledOnce()
    const blocked = setup([
      {
        key: "result",
        title: "Result",
        reasons: null,
        blocked: "Export in progress",
      },
    ])
    await act(() => blocked.hook.result.current.decide("discard"))
    expect(blocked.close).not.toHaveBeenCalled()
    expect(blocked.hook.result.current.review?.notice).toContain(
      "Export in progress"
    )
  })
})
