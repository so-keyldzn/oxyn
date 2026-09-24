import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, within } from "storybook/test"

import { AssistantEntryButton } from "./assistant-entry-button"
import { destinations, enabledEntry } from "./assistant-fixtures"
import { NO_USABLE_DESTINATION } from "@/features/assistant/availability"

const meta = {
  title: "Oxyn/Assistant/EntryButton",
  component: AssistantEntryButton,
  args: { entry: enabledEntry, pressed: false, onPressedChange: fn() },
  decorators: [
    (Story) => (
      <div className="p-4">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof AssistantEntryButton>

export default meta
type Story = StoryObj<typeof meta>

export const Enabled: Story = {
  play: async ({ canvasElement, args }) => {
    await userEvent.click(
      within(canvasElement).getByRole("button", { name: "Ask AI" })
    )
    await expect(args.onPressedChange).toHaveBeenCalledWith(
      true,
      expect.anything()
    )
  },
}

export const Open: Story = { args: { pressed: true } }

export const Unavailable: Story = {
  args: {
    entry: {
      status: "disabled",
      reason: NO_USABLE_DESTINATION,
      destinations,
    },
  },
  play: async ({ canvasElement }) => {
    // Visible, and it says why — once, as a description, not in its name.
    const button = within(canvasElement).getByRole("button", { name: "Ask AI" })
    await expect(button).toHaveAccessibleDescription(/Unavailable:/)
    await expect(button).not.toHaveTextContent(/Unavailable/)
  },
}

export const WithTheTierBeside: Story = {
  args: { tier: "sampled", reach: "remote" },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    // Read before speaking: the tier sits next to the entry.
    await expect(canvas.getByText("Sampled · Cloud")).toBeVisible()
    await expect(canvas.getByRole("button", { name: "Ask AI" })).toBeVisible()
  },
}

export const NoProviderDeclared: Story = {
  args: { entry: { status: "absent" }, tier: "metadata" },
  play: async ({ canvasElement }) => {
    // No button, no badge, no invitation (docs/UX-SPEC.md).
    await expect(within(canvasElement).queryByRole("button")).toBeNull()
    await expect(canvasElement.querySelector("[data-privacy-tier]")).toBeNull()
  },
}
