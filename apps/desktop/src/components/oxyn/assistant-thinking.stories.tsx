import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, userEvent, within } from "storybook/test"

import { AssistantThinking } from "./assistant-thinking"
import type { ThinkingEntry } from "@/features/assistant/transcript"

const entry: ThinkingEntry = {
  kind: "thinking",
  key: "th-0",
  text: "The question asks for active clients. `clients` has `deleted_at`, so\nactive means `deleted_at IS NULL`.",
  redacted: false,
  elapsedMs: 4200,
}

const meta = {
  title: "Oxyn/Assistant/Thinking",
  component: AssistantThinking,
  args: { entry },
  decorators: [
    (Story) => (
      <div className="max-w-xl p-4">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof AssistantThinking>

export default meta
type Story = StoryObj<typeof meta>

export const Folded: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText("Thought for 4.2 s")).toBeVisible()
    // Folded by default: the reasoning is a draft, not the answer.
    await expect(canvas.queryByText(/deleted_at IS NULL/)).toBeNull()
    await userEvent.click(canvas.getByRole("button"))
    await expect(canvas.getByText(/deleted_at IS NULL/)).toBeVisible()
  },
}

export const Streaming: Story = {
  args: { entry: { ...entry, elapsedMs: null } },
  play: async ({ canvasElement }) => {
    await expect(within(canvasElement).getByText("Thinking…")).toBeVisible()
  },
}

export const HiddenByTheProvider: Story = {
  args: { entry: { ...entry, text: "", redacted: true } },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    // Saying it reasoned and shows nothing beats saying nothing at all.
    await expect(
      canvas.getByText("Thought for 4.2 s · hidden by the provider")
    ).toBeVisible()
    await expect(canvas.queryByRole("button")).toBeNull()
  },
}

export const LongAndUnfolded: Story = {
  args: {
    entry: {
      ...entry,
      elapsedMs: 96_000,
      text: Array.from(
        { length: 60 },
        (_, line) => `Step ${line + 1}: consider the join once more.`
      ).join("\n"),
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText("Thought for 96 s")).toBeVisible()
    await userEvent.click(canvas.getByRole("button"))
    await expect(canvas.getByText(/Step 60/)).toBeInTheDocument()
  },
}
