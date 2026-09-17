import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, within } from "storybook/test"

import { AssistantAnswerActions } from "./assistant-answer-actions"

const meta = {
  title: "Oxyn/Assistant/AnswerActions",
  component: AssistantAnswerActions,
  args: {
    text: "There are 1,204 active clients.",
    canRegenerate: true,
    onRegenerate: fn(),
    onCopy: fn(() => true),
  },
  decorators: [
    (Story) => (
      <div className="max-w-xl p-4">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof AssistantAnswerActions>

export default meta
type Story = StoryObj<typeof meta>

export const CopyAndRegenerate: Story = {
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await userEvent.click(canvas.getByRole("button", { name: "Copy answer" }))
    await expect(args.onCopy).toHaveBeenCalledWith(
      "There are 1,204 active clients."
    )
    await expect(
      canvas.getByRole("button", { name: "Copy answer: copied" })
    ).toBeVisible()
    await userEvent.click(
      canvas.getByRole("button", { name: "Regenerate answer" })
    )
    await expect(args.onRegenerate).toHaveBeenCalledOnce()
  },
}

export const CopyRefusedByTheBrowser: Story = {
  args: { onCopy: fn(() => false) },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(canvas.getByRole("button", { name: "Copy answer" }))
    // A refused clipboard must not show a tick.
    await expect(
      canvas.getByRole("button", { name: "Copy answer" })
    ).toBeVisible()
  },
}

export const WhileSomethingRuns: Story = {
  args: { canRegenerate: false },
  play: async ({ canvasElement }) => {
    await expect(
      within(canvasElement).getByRole("button", { name: "Regenerate answer" })
    ).toBeDisabled()
  },
}
