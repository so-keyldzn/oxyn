import * as React from "react"
import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

import { SettingsDialogView } from "./settings-dialog-view"
import { DEFAULT_PREFERENCES } from "./preferences"
import { summaries } from "@/components/oxyn/connection-fixtures"
import { ConnectionManager } from "@/components/oxyn/connection-manager"
import {
  expectContainedInFrame,
  openFrame,
} from "@/components/oxyn/frame-overflow"

const meta = {
  title: "Screens/SettingsDialog",
  component: SettingsDialogView,
  args: {
    open: true,
    onOpenChange: fn(),
    section: "appearance",
    onSectionChange: fn(),
    preferences: DEFAULT_PREFERENCES,
    save: { status: "idle" },
    onReloadPreferences: fn(),
    onPreferencesChange: fn(),
    onRetrySave: fn(),
    connections: (
      <ConnectionManager
        connections={summaries}
        onEdit={() => undefined}
        onDelete={() => undefined}
      />
    ),
  },
} satisfies Meta<typeof SettingsDialogView>

export default meta
type Story = StoryObj<typeof meta>

export const Appearance: Story = {
  play: async ({ args }) => {
    const dialog = within(document.body)
    await userEvent.click(await dialog.findByRole("button", { name: "Light" }))
    await expect(args.onPreferencesChange).toHaveBeenCalledWith({
      theme: "light",
    })
    await userEvent.click(dialog.getByRole("tab", { name: "Formats" }))
    await expect(args.onSectionChange).toHaveBeenCalledWith("formats")
  },
}

export const Formats: Story = { args: { section: "formats" } }

export const Connections: Story = { args: { section: "connections" } }

export const InjectedSection: Story = {
  args: {
    section: "ai",
    sections: [
      {
        id: "ai",
        label: "AI providers",
        content: <p className="text-sm">No provider declared.</p>,
      },
    ],
  },
  play: async () => {
    const dialog = within(document.body)
    const tab = await dialog.findByRole("tab", {
      name: "AI providers",
      selected: true,
    })
    await waitFor(() => expect(tab).toBeVisible())
  },
}

/**
 * Long connection names and locations, and a section with a long label: the
 * tabs, the list and the section stay inside the frame. In a compact window
 * the tabs become a row that scrolls sideways — by design, and only there.
 */
export const LongContentStaysInTheFrame: Story = {
  args: {
    section: "connections",
    sections: [
      {
        id: "ai",
        label: "AI providers and external agents",
        content: <p className="text-sm">No provider declared.</p>,
      },
    ],
    connections: (
      <ConnectionManager
        connections={summaries.map((summary) => ({
          ...summary,
          name: `${summary.name}_analytics_warehouse_production_eu_west_3_read_replica`,
          location: `${summary.location}/analytics_warehouse_production_eu_west_3.cluster_ro`,
        }))}
        onEdit={() => undefined}
        onDelete={() => undefined}
      />
    ),
  },
  play: async () => {
    await expectContainedInFrame(await openFrame("dialog-content"), {
      scrollsSideways: ['[data-slot="tabs-list"]'],
    })
  },
}

/** The same, in a compact window. */
export const LongContentStaysInTheFrameWhenCompact: Story = {
  ...LongContentStaysInTheFrame,
  globals: { viewport: { value: "mobile1" } },
}

export const UnknownSectionFallsBack: Story = {
  args: { section: "removed-feature" },
  play: async () => {
    const dialog = within(document.body)
    const tab = await dialog.findByRole("tab", {
      name: "Appearance",
      selected: true,
    })
    await waitFor(() => expect(tab).toBeVisible())
  },
}

export const Saving: Story = { args: { save: { status: "saving" } } }

export const SaveFailed: Story = {
  args: {
    save: {
      status: "failed",
      message:
        "Preferences changed elsewhere. Save again to apply your current choices.",
    },
  },
  play: async ({ args }) => {
    const dialog = within(document.body)
    await userEvent.click(
      await dialog.findByRole("button", { name: "Save again" })
    )
    await expect(args.onRetrySave).toHaveBeenCalledOnce()
  },
}

export const PreferencesUnreadable: Story = {
  args: {
    loadError: {
      message: "workspace_preferences: invalid JSON preference payload",
      retryable: false,
    },
  },
}

export const BackendAbsent: Story = {
  args: {
    loadError: {
      message:
        "The Oxyn backend is not available here (read_preferences): open the desktop application.",
      retryable: false,
    },
  },
}

export const EscapeCloses: Story = {
  play: async ({ args }) => {
    const dialog = within(document.body)
    await dialog.findByRole("dialog")
    await userEvent.keyboard("{Escape}")
    await waitFor(() => expect(args.onOpenChange).toHaveBeenCalledWith(false))
  },
}

export const VisibleClose: Story = {
  play: async ({ args }) => {
    // Esc is not the only way out: a visible, focusable Close.
    const dialog = within(document.body)
    const close = await dialog.findByRole("button", { name: "Close" })
    await waitFor(() => expect(close).toBeVisible())
    close.focus()
    await expect(close).toHaveFocus()
    await userEvent.keyboard("{Enter}")
    await waitFor(() => expect(args.onOpenChange).toHaveBeenCalledWith(false))
  },
}

function Controlled() {
  const [preferences, setPreferences] = React.useState(DEFAULT_PREFERENCES)
  return (
    <SettingsDialogView
      {...meta.args}
      preferences={preferences}
      onPreferencesChange={(change) =>
        setPreferences((current) => ({ ...current, ...change }))
      }
    />
  )
}

export const ComfortableLight: Story = {
  globals: { theme: "light" },
  render: () => <Controlled />,
  play: async () => {
    const dialog = within(document.body)
    await userEvent.click(
      await dialog.findByRole("button", { name: /^Comfortable/ })
    )
    // The dialog may still be animating in.
    await waitFor(() =>
      expect(
        dialog.getByRole("button", { name: /^Comfortable/, pressed: true })
      ).toBeVisible()
    )
  },
}

export const UnsavedConnectionEdit: Story = {
  args: { section: "connections", unsavedEdit: true },
  play: async ({ args }) => {
    const body = within(document.body)
    await body.findByRole("dialog")

    // Esc asks, and « Keep editing » is the default.
    await userEvent.keyboard("{Escape}")
    const keep = await body.findByRole("button", { name: "Keep editing" })
    await waitFor(() => expect(keep).toHaveFocus())
    await userEvent.keyboard("{Enter}")
    await waitFor(() => expect(body.queryByRole("alertdialog")).toBeNull())
    await expect(args.onOpenChange).not.toHaveBeenCalled()

    // Close asks the same.
    await userEvent.click(body.getByRole("button", { name: "Close" }))
    await waitFor(() =>
      expect(body.getByRole("button", { name: "Keep editing" })).toHaveFocus()
    )
    await userEvent.click(body.getByRole("button", { name: "Keep editing" }))
    await waitFor(() => expect(body.queryByRole("alertdialog")).toBeNull())
    await expect(args.onOpenChange).not.toHaveBeenCalled()

    // Another section asks too; Discard goes there.
    await userEvent.click(body.getByRole("tab", { name: "Appearance" }))
    await expect(args.onSectionChange).not.toHaveBeenCalled()
    await userEvent.click(await body.findByRole("button", { name: "Discard" }))
    await expect(args.onSectionChange).toHaveBeenCalledWith("appearance")
  },
}

export const UnsavedConnectionEditDiscardCloses: Story = {
  args: { section: "connections", unsavedEdit: true },
  play: async ({ args }) => {
    const body = within(document.body)
    await body.findByRole("dialog")
    await userEvent.keyboard("{Escape}")
    await userEvent.click(await body.findByRole("button", { name: "Discard" }))
    await waitFor(() => expect(args.onOpenChange).toHaveBeenCalledWith(false))
  },
}
