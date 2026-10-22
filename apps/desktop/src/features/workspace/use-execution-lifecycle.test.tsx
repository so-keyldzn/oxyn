import { act, renderHook } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import type { CommandOutcome } from "@/lib/ipc/types"
import { executionProgress } from "@/lib/ipc/events"

const calls = vi.hoisted(() => ({
  cancel: vi.fn(() => Promise.resolve(true)),
  forgetResult: vi.fn(() => Promise.resolve()),
  decide: vi.fn(() => Promise.resolve({ type: "denied", reason: "no" })),
  newCommandId: vi.fn(),
}))

vi.mock("@/lib/ipc/client", () => ({
  BackendError: class extends Error {},
  newCommandId: calls.newCommandId,
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
  rowLimitUnverified: false,
}

afterEach(() => {
  executionProgress.setState(() => ({}))
  vi.clearAllMocks()
})

beforeEach(() => calls.newCommandId.mockReturnValue("run-1"))

describe("useExecution lifecycle", () => {
  it("cancels a running command when its view goes, and forgets its late result", async () => {
    let answer: (outcome: CommandOutcome) => void = () => undefined
    const { result, unmount } = renderHook(() =>
      useExecution({ connection: "c", serverCancel: true })
    )
    act(() =>
      result.current.start(
        () =>
          new Promise<CommandOutcome>((resolve) => {
            answer = resolve
          })
      )
    )
    expect(executionProgress.state).toHaveProperty("run-1")
    unmount()
    expect(calls.cancel).toHaveBeenCalledWith("run-1")
    expect(executionProgress.state).not.toHaveProperty("run-1")

    await act(async () => answer(executed))
    expect(calls.forgetResult).toHaveBeenCalledWith("late-result")
  })

  it("forgets a running command when the console is reset", () => {
    const { result } = renderHook(() =>
      useExecution({ connection: "c", serverCancel: true })
    )
    act(() => result.current.start(() => new Promise(() => undefined)))
    expect(executionProgress.state).toHaveProperty("run-1")

    act(() => result.current.reset())

    expect(calls.cancel).toHaveBeenCalledWith("run-1")
    expect(executionProgress.state).not.toHaveProperty("run-1")
  })

  it("forgets the replaced run when its late answer settles", async () => {
    let firstAnswer: (outcome: CommandOutcome) => void = () => undefined
    calls.newCommandId
      .mockReturnValueOnce("first")
      .mockReturnValueOnce("second")
    const { result } = renderHook(() =>
      useExecution({ connection: "c", serverCancel: true })
    )
    act(() =>
      result.current.start(
        () =>
          new Promise<CommandOutcome>((resolve) => {
            firstAnswer = resolve
          })
      )
    )
    act(() => result.current.start(() => new Promise(() => undefined)))

    expect(executionProgress.state).not.toHaveProperty("first")
    expect(executionProgress.state).toHaveProperty("second")
    await act(async () => firstAnswer(executed))

    expect(executionProgress.state).not.toHaveProperty("first")
    expect(executionProgress.state).toHaveProperty("second")
  })

  it("refuses an approval still pending when its view goes", async () => {
    const { result, unmount } = renderHook(() =>
      useExecution({ connection: "c", serverCancel: true })
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
    const { result } = renderHook(() =>
      useExecution({ connection: "c", serverCancel: true })
    )
    act(() => result.current.start(() => new Promise(() => undefined)))
    act(() => result.current.cancel())
    act(() => result.current.cancel())
    expect(calls.cancel).toHaveBeenCalledTimes(1)
    expect(result.current.cancelling).toBe(true)
  })
})
