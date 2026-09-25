import { act, renderHook } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import {
  DEFAULT_PREFERENCES,
  preferencesStore,
} from "@/features/settings/preferences"
import { usePanelPreferences } from "@/features/workspace/use-panel-preferences"
import { BackendError } from "@/lib/ipc/client"

const writePreferences = vi.hoisted(() => vi.fn())
const toastAdd = vi.hoisted(() => vi.fn())

vi.mock("@/lib/ipc/settings", () => ({
  settingsBackend: { writePreferences },
}))

vi.mock("@/components/ui/toast", () => ({
  toast: { add: toastAdd },
}))

function snapshot(sidebarCollapsed: boolean, inspectorOpen: boolean) {
  preferencesStore.setState((state) => ({
    ...state,
    preferences: { ...DEFAULT_PREFERENCES, sidebarCollapsed, inspectorOpen },
    loaded: true,
    save: { status: "idle" },
  }))
}

beforeEach(() => {
  writePreferences.mockResolvedValue({})
})

afterEach(() => {
  vi.clearAllMocks()
})

describe("usePanelPreferences", () => {
  it("starts from the saved snapshot at wide width", () => {
    snapshot(true, false)
    const { result } = renderHook(() => usePanelPreferences(false))
    expect(result.current.sidebarOpen).toBe(false)
    expect(result.current.asideOpen).toBe(false)

    act(() => snapshot(false, true))
    expect(result.current.sidebarOpen).toBe(true)
    expect(result.current.asideOpen).toBe(true)
  })

  it("saves each wide-width toggle, and only a change", async () => {
    snapshot(false, true)
    const { result } = renderHook(() => usePanelPreferences(false))

    await act(async () => result.current.setSidebarOpen(false))
    expect(writePreferences).toHaveBeenLastCalledWith({
      sidebarCollapsed: true,
    })
    expect(result.current.sidebarOpen).toBe(false)

    await act(async () => result.current.setAsideOpen(false))
    expect(writePreferences).toHaveBeenLastCalledWith({ inspectorOpen: false })
    expect(result.current.asideOpen).toBe(false)

    await act(async () => result.current.setAsideOpen(false))
    expect(writePreferences).toHaveBeenCalledTimes(2)
  })

  it("keeps the column closed and writes nothing below 1200 px", async () => {
    snapshot(false, true)
    const { result, rerender } = renderHook(
      ({ compact }) => usePanelPreferences(compact),
      { initialProps: { compact: true } }
    )
    expect(result.current.asideOpen).toBe(false)

    await act(async () => {
      result.current.setAsideOpen(true)
      result.current.setSidebarOpen(false)
    })
    expect(result.current.asideOpen).toBe(true)
    expect(writePreferences).not.toHaveBeenCalled()

    // Widening gives the wide preference back; narrowing again finds the
    // overlay closed.
    rerender({ compact: false })
    expect(result.current.asideOpen).toBe(true)
    expect(result.current.sidebarOpen).toBe(true)
    rerender({ compact: true })
    expect(result.current.asideOpen).toBe(false)
  })

  it("says a failed save and offers to save again", async () => {
    snapshot(false, true)
    writePreferences.mockRejectedValueOnce(
      new BackendError({
        message: "database is locked",
        retryable: true,
      })
    )
    const { result } = renderHook(() => usePanelPreferences(false))

    await act(async () => result.current.setSidebarOpen(false))
    // Applied here all the same.
    expect(result.current.sidebarOpen).toBe(false)
    expect(preferencesStore.state.save).toEqual({
      status: "failed",
      message: "database is locked",
    })
    expect(toastAdd).toHaveBeenCalledWith(
      expect.objectContaining({
        description: "database is locked",
        actionProps: expect.objectContaining({ children: "Save again" }),
      })
    )
  })
})
