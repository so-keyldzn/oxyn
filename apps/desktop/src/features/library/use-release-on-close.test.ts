import { renderHook } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"

import { useReleaseOnClose } from "@/features/library/use-release-on-close"

const forgetResult = vi.hoisted(() => vi.fn(() => Promise.resolve()))

vi.mock("@/lib/ipc/results", () => ({ results: { forgetResult } }))

afterEach(() => {
  vi.clearAllMocks()
})

describe("a view reading a retained result", () => {
  it("releases its reader once, when it closes", () => {
    const view = renderHook(() => useReleaseOnClose("result"))
    view.rerender()
    expect(forgetResult).not.toHaveBeenCalled()

    view.unmount()
    expect(forgetResult).toHaveBeenCalledTimes(1)
    expect(forgetResult).toHaveBeenCalledWith("result")
  })

  it("releases nothing it did not open", () => {
    const view = renderHook(() => useReleaseOnClose(null))
    view.unmount()
    expect(forgetResult).not.toHaveBeenCalled()
  })

  it("releases only its own reader when two views read the same result", () => {
    const first = renderHook(() => useReleaseOnClose("result"))
    const second = renderHook(() => useReleaseOnClose("result"))

    first.unmount()
    expect(forgetResult).toHaveBeenCalledTimes(1)
    second.unmount()
    expect(forgetResult).toHaveBeenCalledTimes(2)
  })

  it("releases the previous result when the view moves to another", () => {
    const view = renderHook(({ result }) => useReleaseOnClose(result), {
      initialProps: { result: "first" },
    })
    view.rerender({ result: "second" })
    expect(forgetResult).toHaveBeenCalledWith("first")
    view.unmount()
    expect(forgetResult).toHaveBeenLastCalledWith("second")
    expect(forgetResult).toHaveBeenCalledTimes(2)
  })
})
