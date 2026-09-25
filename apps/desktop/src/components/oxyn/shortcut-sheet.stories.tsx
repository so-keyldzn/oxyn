import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, screen, within } from "storybook/test"

import { ShortcutSheet } from "./shortcut-sheet"
import { shortcutSheet } from "@/lib/actions/listings"
import { manifest } from "@/lib/actions/manifest"

const meta = {
  title: "Oxyn/ShortcutSheet",
  component: ShortcutSheet,
  args: {
    open: true,
    onOpenChange: fn(),
    sections: shortcutSheet(manifest, "other"),
  },
} satisfies Meta<typeof ShortcutSheet>

export default meta
type Story = StoryObj<typeof meta>

/** Windows and Linux: Ctrl, and ⌘⌥B become Ctrl+Shift+B. */
export const WindowsAndLinux: Story = {
  play: async () => {
    const dialog = await screen.findByRole("dialog", {
      name: "Keyboard shortcuts",
    })
    const editor = within(dialog).getByRole("region", {
      name: "SQL editor",
    })
    await expect(editor).toHaveTextContent("Toggle comment")
    const everywhere = within(dialog).getByRole("region", {
      name: "Everywhere",
    })
    await expect(everywhere).toHaveTextContent("Toggle side panel")
    await expect(everywhere).toHaveTextContent("CtrlShiftB")
  },
}

/** macOS: the same sheet, in Apple's key caps. */
export const MacOS: Story = {
  args: { sections: shortcutSheet(manifest, "mac") },
  play: async () => {
    const dialog = await screen.findByRole("dialog", {
      name: "Keyboard shortcuts",
    })
    await expect(dialog).toHaveTextContent("⌘")
  },
}
