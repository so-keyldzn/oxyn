import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

import { AssistantCopyButton } from "./assistant-copy-button"

/** The prop's own signature: a clipboard answers now or later. */
type Copy = (text: string) => boolean | Promise<boolean>

const meta = {
  title: "Oxyn/Assistant/CopyButton",
  component: AssistantCopyButton,
  args: {
    text: "SELECT count(*) FROM invoices;",
    label: "Copy answer",
    onCopy: fn<Copy>(() => true),
  },
  decorators: [
    (Story) => (
      <div className="p-4">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof AssistantCopyButton>

export default meta
type Story = StoryObj<typeof meta>

export const Ready: Story = {
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    const button = canvas.getByRole("button", { name: "Copy answer" })
    await userEvent.click(button)
    await expect(args.onCopy).toHaveBeenCalledWith(
      "SELECT count(*) FROM invoices;"
    )
    // The tick says the clipboard took it, and the name says so too: a user
    // who cannot see the icon still hears the confirmation.
    await waitFor(() =>
      expect(
        canvas.getByRole("button", { name: "Copy answer: copied" })
      ).toBeVisible()
    )
  },
}

export const ClipboardRefused: Story = {
  args: { onCopy: fn<Copy>(() => false) },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(canvas.getByRole("button", { name: "Copy answer" }))
    // A webview without clipboard permission must not show a tick: the user
    // would paste the previous content believing it worked.
    await expect(canvas.queryByRole("button", { name: /copied/ })).toBeNull()
  },
}

export const SlowClipboard: Story = {
  args: {
    onCopy: fn<Copy>(
      () =>
        new Promise<boolean>((resolve) => {
          window.setTimeout(() => resolve(true), 400)
        })
    ),
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const button = canvas.getByRole("button", { name: "Copy answer" })
    // Double click: the second one resolves the same way, no flicker of state.
    await userEvent.dblClick(button)
    await waitFor(() =>
      expect(
        canvas.getByRole("button", { name: "Copy answer: copied" })
      ).toBeVisible()
    )
  },
}

export const NothingToCopy: Story = {
  args: { text: "" },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    // An answer still streaming has nothing to copy yet.
    await expect(
      canvas.getByRole("button", { name: "Copy answer" })
    ).toBeDisabled()
  },
}

export const OnACodeBlock: Story = {
  args: { label: "Copy code", size: "icon-sm", text: "VACUUM FULL invoices;" },
}
