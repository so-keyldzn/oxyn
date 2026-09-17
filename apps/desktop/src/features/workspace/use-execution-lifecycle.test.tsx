import { act, renderHook } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"

import type { CommandOutcome } from "@/lib/ipc/types"

const calls = vi.hoisted(() => ({
  cancel: vi.fn(() => Promise.resolve(true)),
  forgetResult: vi.fn(() => Promise.resolve()),
  decide: vi.fn(() => Promise.resolve({ type: "denied", reason: "no" })),
}))

vi.mock("@/lib/ipc/client", () => ({
  BackendError: class extends Error {},
  newCommandId: () => "run-1",
  backend: calls,
}))

const { useExecution } = await import("./use-execution")

const executed: CommandOutcome = {
  type: "executed",
  result: "late-result",
  columns: [],
  rows: 1,
  elapsedMs: 3,
  complete: true,
  cancelled: false,
  truncated: false,
}

afterEach(() => vi.clearAllMocks())

describe("useExecution lifecycle", () => {
  it("cancels a running command when its view goes, and forgets its late result", async () => {
    let answer: (outcome: CommandOutcome) => void = () => undefined
    const { result, unmount } = renderHook(() =>
      useExecution({ serverCancel: true })
    )
    act(() =>
      result.current.start(
        () =>
          new Promise<CommandOutcome>((resolve) => {
            answer = resolve
          })
      )
    )
    unmount()
    expect(calls.cancel).toHaveBeenCalledWith("run-1")

    await act(async () => answer(executed))
    expect(calls.forgetResult).toHaveBeenCalledWith("late-result")
  })

  it("refuses an approval still pending when its view goes", async () => {
    const { result, unmount } = renderHook(() =>
      useExecution({ serverCancel: true })
    )
    await act(async () =>
      result.current.start(() =>
        Promise.resolve<CommandOutcome>({
          type: "needsApproval",
          command: "held",
          reason: "production",
          preview: null,
        })
      )
    )
    expect(result.current.approval?.command).toBe("held")
    unmount()
    expect(calls.decide).toHaveBeenCalledWith("held", false)
  })

  it("sends Stop once, however often it is pressed", () => {
    const { result } = renderHook(() => useExecution({ serverCancel: true }))
    act(() => result.current.start(() => new Promise(() => undefined)))
    act(() => result.current.cancel())
    act(() => result.current.cancel())
    expect(calls.cancel).toHaveBeenCalledTimes(1)
    expect(result.current.cancelling).toBe(true)
  })
})
