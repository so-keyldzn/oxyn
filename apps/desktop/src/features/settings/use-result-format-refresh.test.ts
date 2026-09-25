import { QueryClient, QueryObserver } from "@tanstack/react-query"
import { act, renderHook } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"

import { RESULT_PAGE_QUERY } from "@/components/oxyn/result-grid"
import {
  DEFAULT_PREFERENCES,
  changePreferences,
  preferencesStore,
} from "@/features/settings/preferences"
import { useResultFormatRefresh } from "@/features/settings/use-result-format-refresh"
import { BackendError } from "@/lib/ipc/client"

const writePreferences = vi.hoisted(() => vi.fn())

vi.mock("@/lib/ipc/settings", () => ({
  settingsBackend: { writePreferences },
}))

function reset() {
  preferencesStore.setState((state) => ({
    ...state,
    preferences: DEFAULT_PREFERENCES,
    save: { status: "idle" },
  }))
}

afterEach(() => {
  vi.clearAllMocks()
})

/**
 * A grid holding one page: the backend formats it with the options in force
 * when it is read, as `read_result_page` does.
 */
async function gridHoldingOnePage() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  let grouped = false
  const readPage = vi.fn(() =>
    Promise.resolve(grouped ? "4,823,917" : "4823917")
  )
  const observer = new QueryObserver(client, {
    queryKey: [RESULT_PAGE_QUERY, "result", 200, 0, 1],
    queryFn: readPage,
    staleTime: Number.POSITIVE_INFINITY,
  })
  const unsubscribe = observer.subscribe(() => undefined)
  await vi.waitFor(() =>
    expect(observer.getCurrentResult().data).toBe("4823917")
  )
  return {
    client,
    readPage,
    observer,
    unsubscribe,
    groupOnTheBackend: () => {
      grouped = true
    },
  }
}

describe("result pages after a format change", () => {
  it("are read again from the retained result once the change is saved", async () => {
    reset()
    const grid = await gridHoldingOnePage()
    renderHook(() => useResultFormatRefresh(grid.client))

    writePreferences.mockImplementation(() => {
      grid.groupOnTheBackend()
      return Promise.resolve({})
    })
    await act(async () => {
      await changePreferences({ groupThousands: true })
    })

    await vi.waitFor(() =>
      expect(grid.observer.getCurrentResult().data).toBe("4,823,917")
    )
    // One read to fill the page, one to reformat it: a page read, never SQL.
    expect(grid.readPage).toHaveBeenCalledTimes(2)
    grid.unsubscribe()
  })

  it.each([
    { binaryDisplay: "base64" as const },
    { cellMaxChars: 64 },
    { nullText: "NULL" },
  ])("are refreshed for every formatting preference: %o", async (change) => {
    reset()
    const grid = await gridHoldingOnePage()
    renderHook(() => useResultFormatRefresh(grid.client))
    writePreferences.mockResolvedValue({})

    await act(async () => {
      await changePreferences(change)
    })

    await vi.waitFor(() => expect(grid.readPage).toHaveBeenCalledTimes(2))
    grid.unsubscribe()
  })

  it("stay as they are for a change that formats nothing", async () => {
    reset()
    const grid = await gridHoldingOnePage()
    renderHook(() => useResultFormatRefresh(grid.client))
    writePreferences.mockResolvedValue({})

    await act(async () => {
      await changePreferences({ theme: "light" })
    })

    expect(
      grid.client.getQueryState([RESULT_PAGE_QUERY, "result", 200, 0, 1])
        ?.isInvalidated
    ).toBe(false)
    expect(grid.readPage).toHaveBeenCalledTimes(1)
    grid.unsubscribe()
  })

  it("stay as they are when the backend did not take the change", async () => {
    reset()
    const grid = await gridHoldingOnePage()
    renderHook(() => useResultFormatRefresh(grid.client))
    writePreferences.mockRejectedValue(
      new BackendError({ message: "disk full", retryable: false })
    )

    await act(async () => {
      await changePreferences({ groupThousands: true })
    })

    // The backend still formats the old way: the held pages agree with it.
    expect(grid.readPage).toHaveBeenCalledTimes(1)
    expect(grid.observer.getCurrentResult().data).toBe("4823917")
    grid.unsubscribe()
  })
})
