import { act, cleanup, renderHook } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { LIST_DEBOUNCE_MS, useDraftModels } from "./use-draft-models"
import type { ListDraftModels } from "./use-draft-models"
import type { ProbeInputs } from "./model-picker-model"
import type { ModelListing } from "@/lib/ipc/ai"

const input: ProbeInputs = {
  kind: "openai_compatible",
  endpoint: "http://localhost:11434/v1",
  key: "",
  editing: null,
}

function answer(fetchedAtMs = 1): ModelListing {
  return { status: "ok", models: [], cached: false, fetchedAtMs }
}

function pendingListing() {
  let resolve: (value: ModelListing) => void
  const promise = new Promise<ModelListing>((done) => {
    resolve = done
  })
  return { promise, resolve: (value: ModelListing) => resolve(value) }
}

async function advance(ms = LIST_DEBOUNCE_MS) {
  await act(() => vi.advanceTimersByTimeAsync(ms))
}

beforeEach(() => vi.useFakeTimers())
afterEach(() => {
  cleanup()
  vi.useRealTimers()
})

describe("listing models for the provider being typed", () => {
  it("lists only the final endpoint after typing quiets down", async () => {
    const list = vi.fn<ListDraftModels>().mockResolvedValue(answer())
    const hook = renderHook((inputs) => useDraftModels(inputs, list), {
      initialProps: input,
    })
    await advance(200)
    hook.rerender({ ...input, endpoint: "http://localhost:1234/v1" })
    await advance(LIST_DEBOUNCE_MS - 1)
    expect(list).not.toHaveBeenCalled()
    await advance(1)
    expect(list).toHaveBeenCalledExactlyOnceWith(
      {
        id: null,
        kind: input.kind,
        baseUrl: "http://localhost:1234/v1",
        key: null,
      },
      false
    )
    expect(hook.result.current.models).toEqual(answer())
  })

  it("ignores an older response and bypasses the cache after a key change", async () => {
    const first = pendingListing()
    const second = pendingListing()
    const list = vi
      .fn<ListDraftModels>()
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise)
    const initial = { ...input, key: "synthetic-first-key" }
    const hook = renderHook((inputs) => useDraftModels(inputs, list), {
      initialProps: initial,
    })
    await advance()
    hook.rerender({ ...initial, key: "synthetic-second-key" })
    expect(hook.result.current.models.status).toBe("loading")
    await advance()
    expect(list).toHaveBeenLastCalledWith(
      expect.objectContaining({ key: "synthetic-second-key" }),
      true
    )
    await act(async () => {
      second.resolve(answer(2))
    })
    await act(async () => {
      first.resolve(answer(1))
    })
    expect(hook.result.current.models).toEqual(answer(2))
  })

  it("cancels a pending listing when the provider becomes incomplete", async () => {
    const list = vi.fn<ListDraftModels>().mockResolvedValue(answer())
    const hook = renderHook<
      ReturnType<typeof useDraftModels>,
      ProbeInputs | null
    >((inputs) => useDraftModels(inputs, list), {
      initialProps: input,
    })
    hook.rerender(null)
    await advance()
    expect(list).not.toHaveBeenCalled()
    expect(hook.result.current.models.status).toBe("idle")
  })

  it("cancels a pending listing on unmount", async () => {
    const list = vi.fn<ListDraftModels>().mockResolvedValue(answer())
    const hook = renderHook(() => useDraftModels(input, list))
    hook.unmount()
    await advance()
    expect(list).not.toHaveBeenCalled()
  })

  it("lets a manual refresh supersede the pending automatic listing", async () => {
    const list = vi.fn<ListDraftModels>().mockResolvedValue(answer())
    const hook = renderHook(() => useDraftModels(input, list))
    await act(async () => {
      hook.result.current.refresh()
    })
    await advance()
    expect(list).toHaveBeenCalledExactlyOnceWith(
      expect.objectContaining({ baseUrl: input.endpoint }),
      true
    )
    expect(hook.result.current.models).toEqual(answer())
  })

  it("keeps a failed manual connection test visible", async () => {
    const failed: ModelListing = {
      status: "failed",
      reason: "unreachable",
      message: "Provider unavailable",
    }
    const list = vi.fn<ListDraftModels>().mockResolvedValue(failed)
    const hook = renderHook(() => useDraftModels(input, list))
    await act(async () => {
      hook.result.current.testConnection()
    })
    await advance()
    expect(list).toHaveBeenCalledOnce()
    expect(hook.result.current.test).toEqual({
      status: "failed",
      message: "Provider unavailable",
    })
  })
})
