import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect } from "storybook/test"

import { AssistantPrunedNote } from "./assistant-pruned-note"

const meta = {
  title: "Oxyn/Assistant/PrunedNote",
  component: AssistantPrunedNote,
  decorators: [
    (Story) => (
      <div className="w-80 p-3">
        <Story />
      </div>
    ),
  ],
  args: {
    pruned: {
      conversations: 3,
      maxConversations: 200,
      maxAgeDays: 90,
      maxBytes: 32 * 1024 * 1024,
    },
  },
} satisfies Meta<typeof AssistantPrunedNote>

export default meta
type Story = StoryObj<typeof meta>

/** How many went, and by which rule — never only in a log. */
export const Pruned: Story = {
  play: async ({ canvas }) => {
    await expect(
      canvas.getByText(
        "Oxyn removed 3 conversations when it started. It keeps the 200 most recent, within 32 MiB, and removes those idle for 90 days."
      )
    ).toBeVisible()
  },
}

export const OneWithoutAgeBound: Story = {
  args: {
    pruned: {
      conversations: 1,
      maxConversations: 200,
      maxAgeDays: null,
      maxBytes: 32 * 1024 * 1024,
    },
  },
  play: async ({ canvas }) => {
    await expect(canvas.getByText(/removed 1 conversation when/)).toBeVisible()
    await expect(canvas.queryByText(/idle/)).toBeNull()
  },
}
