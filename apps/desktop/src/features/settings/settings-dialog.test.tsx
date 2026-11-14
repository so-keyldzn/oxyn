import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { Button } from "@/components/ui/button"
import { SettingsDialog } from "./settings-dialog"
import { closeSettings, openSettings } from "./settings-dialog-state"
import { invoke } from "@/lib/actions/registry"

vi.mock("./connections-settings", () => ({ ConnectionsSettings: () => null }))
vi.mock("./about-section", () => ({ AboutSection: () => null }))
vi.mock("@/features/updates/updates-section", () => ({
  UpdatesSection: () => null,
}))

beforeEach(() => {
  openSettings("appearance")
  closeSettings()
})
afterEach(() => {
  cleanup()
  closeSettings()
})

function SettingsHost() {
  return (
    <>
      <Button onClick={() => void invoke("app.settings", "button")}>
        Open settings
      </Button>
      <SettingsDialog />
    </>
  )
}

describe("opening Settings", () => {
  it("opens through the button and keyboard action after closing", async () => {
    render(<SettingsHost />)
    fireEvent.click(screen.getByRole("button", { name: "Open settings" }))
    expect(await screen.findByRole("dialog", { name: "Settings" })).toBeTruthy()
    fireEvent.click(screen.getByRole("button", { name: "Close" }))
    await waitFor(() =>
      expect(screen.queryByRole("dialog", { name: "Settings" })).toBeNull()
    )
    act(() => {
      invoke("app.settings", "keyboard")
    })
    expect(await screen.findByRole("dialog", { name: "Settings" })).toBeTruthy()
  })

  it("keeps an opener valid when the view remounts, including its target section", async () => {
    const openFromAnotherFeature = openSettings
    const first = render(<SettingsHost />)
    first.unmount()
    render(<SettingsHost />)
    act(() => openFromAnotherFeature("formats"))
    const dialog = await screen.findByRole("dialog", { name: "Settings" })
    expect(dialog.textContent).toContain("Missing value marker")
    expect(
      screen.getByRole("tab", { name: "Formats" }).getAttribute("aria-selected")
    ).toBe("true")
  })
})
