import { act, render } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"

import { CellValue } from "@/components/oxyn/cell-value"
import { ValuePageDialog } from "@/components/oxyn/value-page-dialog"
import {
  DEFAULT_PREFERENCES,
  changePreferences,
  preferencesStore,
} from "@/features/settings/preferences"
import { PreferredNullText } from "@/features/settings/preferred-null-text"

const writePreferences = vi.hoisted(() => vi.fn())

vi.mock("@/lib/ipc/settings", () => ({
  settingsBackend: { writePreferences },
}))

afterEach(() => {
  vi.clearAllMocks()
  preferencesStore.setState((state) => ({
    ...state,
    preferences: DEFAULT_PREFERENCES,
    save: { status: "idle" },
  }))
})

/** The text of the one absent-value token under `container`. */
function absentIn(container: HTMLElement) {
  return container.querySelector("[data-null]")?.textContent
}

describe("the saved absent-value marker", () => {
  it("is `∅ NULL` until the preferences say otherwise", () => {
    const { container } = render(
      <PreferredNullText>
        <CellValue cell={null} />
      </PreferredNullText>
    )
    expect(absentIn(container)).toBe("∅ NULL")
  })

  it("redraws a result already shown and draws the next one", async () => {
    writePreferences.mockResolvedValue({ revision: 1 })
    const existing = render(
      <PreferredNullText>
        <CellValue cell={null} />
        <CellValue cell="<MISSING>" />
      </PreferredNullText>
    )

    await act(() => changePreferences({ nullText: "<MISSING>" }))

    expect(absentIn(existing.container)).toBe("<MISSING>")
    // The literal text stays text: only the absent value carries the token.
    expect(existing.container.querySelectorAll("[data-null]")).toHaveLength(1)

    // `SELECT NULL AS missing_value` run after the save.
    const fresh = render(
      <PreferredNullText>
        <CellValue cell={null} />
      </PreferredNullText>
    )
    expect(absentIn(fresh.container)).toBe("<MISSING>")
  })

  it("is what the value inspector shows for an absent value", async () => {
    writePreferences.mockResolvedValue({ revision: 1 })
    await act(() => changePreferences({ nullText: "<MISSING>" }))
    const { baseElement } = render(
      <PreferredNullText>
        <ValuePageDialog
          open
          connectionName="local"
          column="missing_value"
          row={0}
          state={{
            status: "page",
            page: {
              column: "missing_value",
              dataType: "Null",
              isNull: true,
              text: "",
              offset: 0,
              nextOffset: null,
              totalBytes: 0,
            },
          }}
          canGoBack={false}
          onPrevious={() => undefined}
          onNext={() => undefined}
          onRetry={() => undefined}
          onCopyPage={() => undefined}
          onClose={() => undefined}
        />
      </PreferredNullText>
    )
    expect(absentIn(baseElement)).toBe("<MISSING>")
  })
})
