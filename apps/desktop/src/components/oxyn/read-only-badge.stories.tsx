import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect } from "storybook/test"

import { ReadOnlyBadge } from "./read-only-badge"

const meta = {
  title: "Oxyn/ReadOnlyBadge",
  component: ReadOnlyBadge,
} satisfies Meta<typeof ReadOnlyBadge>

export default meta
type Story = StoryObj<typeof meta>

/**
 * The refusal is announced before a statement is written, in words: the lock
 * icon alone would say nothing to a screen reader, nor to someone who does not
 * know the convention.
 */
export const Default: Story = {
  play: async ({ canvas, canvasElement }) => {
    await expect(canvas.getByText("READ ONLY")).toBeVisible()
    await expect(
      canvasElement.querySelector('[data-slot="read-only-badge"]')
    ).not.toBeNull()
  },
}

/** In the dark theme the badge stays legible: axe checks its contrast. */
export const Dark: Story = {
  decorators: [
    (Story) => (
      <div className="dark bg-background p-3 text-foreground">
        <Story />
      </div>
    ),
  ],
  play: async ({ canvas }) => {
    await expect(canvas.getByText("READ ONLY")).toBeVisible()
  },
}
