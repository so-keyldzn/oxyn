import { act, renderHook } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"

import {
  DEFAULT_PREFERENCES,
  preferencesStore,
} from "@/features/settings/preferences"
import { useResultDensity } from "@/features/settings/use-result-density"
import { BackendError } from "@/lib/ipc/client"

const writePreferences = vi.hoisted(() => vi.fn())
const toastAdd = vi.hoisted(() => vi.fn())

vi.mock("@/lib/ipc/settings", () => ({
  settingsBackend: { writePreferences },
}))

vi.mock("@/components/ui/toast", () => ({
  toast: { add: toastAdd },
}))

function snapshot() {
  preferencesStore.setState((state) => ({
    ...state,
    preferences: { ...DEFAULT_PREFERENCES, readingDensity: "compact" },
    save: { status: "idle" },
  }))
}

afterEach(() => {
  vi.clearAllMocks()
})

describe("useResultDensity", () => {
  it("applies the preset at once and saves it with the preferences", async () => {
    snapshot()
    writePreferences.mockResolvedValue({})
    const { result } = renderHook(() => useResultDensity())
    expect(result.current.value).toBe("compact")

    await act(async () => result.current.onChange("comfortable"))
    expect(result.current.value).toBe("comfortable")
    expect(writePreferences).toHaveBeenCalledWith({
      readingDensity: "comfortable",
    })
    expect(preferencesStore.state.save).toEqual({ status: "saved" })

    // The preset in force: nothing to save.
    await act(async () => result.current.onChange("comfortable"))
    expect(writePreferences).toHaveBeenCalledTimes(1)
  })

  it("keeps the preset applied and says it is not saved", async () => {
    snapshot()
    writePreferences.mockRejectedValue(
      new BackendError({ message: "disk full", retryable: false })
    )
    const { result } = renderHook(() => useResultDensity())

    await act(async () => result.current.onChange("comfortable"))
    expect(result.current.value).toBe("comfortable")
    expect(preferencesStore.state.save).toEqual({
      status: "failed",
      message: "disk full",
    })
    expect(toastAdd).toHaveBeenCalledWith(
      expect.objectContaining({
        description: "disk full",
        actionProps: expect.objectContaining({ children: "Save again" }),
      })
    )
  })
})
