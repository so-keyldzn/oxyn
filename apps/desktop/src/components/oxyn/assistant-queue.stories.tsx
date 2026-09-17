import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, within } from "storybook/test"

import { AssistantQueue } from "./assistant-queue"

const meta = {
  title: "Oxyn/Assistant/Queue",
  component: AssistantQueue,
  args: {
    queue: [
      { key: "queued-1", text: "And the same for last month?" },
      { key: "queued-2", text: "Then group it by status." },
    ],
    held: false,
    onSendNow: fn(),
    onRemove: fn(),
  },
  decorators: [
    (Story) => (
      <div className="max-w-xl p-4">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof AssistantQueue>

export default meta
type Story = StoryObj<typeof meta>

export const WaitingForTheTurnToEnd: Story = {
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText(/They approve nothing\./)).toBeVisible()
    // Nothing leaves early, and nothing is sent by hand while it runs.
    await expect(
      canvas.queryByRole("button", { name: /Send queued/ })
    ).toBeNull()
    await userEvent.click(
      canvas.getByRole("button", { name: "Remove queued message 1" })
    )
    await expect(args.onRemove).toHaveBeenCalledWith("queued-1")
  },
}

export const HeldAfterAFailure: Story = {
  args: { held: true },
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText(/the last answer failed/)).toBeVisible()
    await userEvent.click(
      canvas.getByRole("button", { name: "Send queued message 2 now" })
    )
    await expect(args.onSendNow).toHaveBeenCalledWith("queued-2")
  },
}

export const Empty: Story = {
  args: { queue: [] },
  play: async ({ canvasElement }) => {
    await expect(canvasElement.textContent).toBe("")
  },
}

export const LongAndRightToLeft: Story = {
  args: {
    queue: [
      {
        key: "queued-1",
        text: "ما هو عدد العملاء النشطين في قاعدة البيانات هذه؟",
      },
      {
        key: "queued-2",
        text: "Explain ".repeat(40),
      },
    ],
  },
}
