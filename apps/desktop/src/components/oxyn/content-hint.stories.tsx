import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, userEvent, within } from "storybook/test"

import { ContentHint } from "./content-hint"
import { Button } from "@/components/ui/button"

const meta = {
  title: "Oxyn/ContentHint",
  component: ContentHint,
  args: {
    content: "The full object name",
    children: <Button variant="outline">Object</Button>,
  },
} satisfies Meta<typeof ContentHint>
export default meta
type Story = StoryObj<typeof meta>

export const Keyboard: Story = {
  play: async ({ canvas }) => {
    await userEvent.tab()
    await expect(canvas.getByRole("button", { name: "Object" })).toHaveFocus()
    await expect(
      canvas.getByRole("button", { name: "Object" })
    ).toHaveAccessibleDescription("The full object name")
    await expect(
      await within(document.body).findByText("The full object name", {
        selector: '[data-slot="tooltip-content"]',
      })
    ).toHaveTextContent("The full object name")
    await userEvent.keyboard("{Escape}")
  },
}

export const TruncatedText: Story = {
  args: {
    children: <span className="block w-20 truncate">The full object name</span>,
  },
  play: async ({ canvas }) => {
    const text = canvas.getByText("The full object name")
    await userEvent.hover(text)
    await expect(
      await within(document.body).findByText("The full object name", {
        selector: '[data-slot="tooltip-content"]',
      })
    ).toHaveTextContent("The full object name")
    await expect(text).not.toHaveAttribute("title")
    await expect(text).not.toHaveAttribute("tabindex", "0")
    await userEvent.unhover(text)
  },
}

export const NoHint: Story = { args: { content: null } }
