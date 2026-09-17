import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, within } from "storybook/test"

import { AssistantEnding } from "./assistant-ending"

const meta = {
  title: "Oxyn/Assistant/Ending",
  component: AssistantEnding,
  args: {
    ending: { type: "answered", turns: 2, truncated: false, cut: null },
    onContinue: fn(),
  },
  decorators: [
    (Story) => (
      <div className="max-w-xl p-4">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof AssistantEnding>

export default meta
type Story = StoryObj<typeof meta>

export const Answered: Story = { args: { onContinue: undefined } }

export const Truncated: Story = {
  args: {
    ending: { type: "answered", turns: 3, truncated: true, cut: "tokenLimit" },
  },
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText(/It is not finished\./)).toBeVisible()
    await userEvent.click(canvas.getByRole("button", { name: "Continue" }))
    await expect(args.onContinue).toHaveBeenCalledOnce()
  },
}

export const CutByTheContextWindow: Story = {
  args: {
    ending: {
      type: "answered",
      turns: 4,
      truncated: true,
      cut: "contextWindow",
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    // Not « the token limit »: raising it would not help.
    await expect(canvas.getByText(/context window/)).toBeVisible()
    await expect(canvas.queryByText(/token limit\./)).toBeNull()
  },
}

export const ProviderErrorMidAnswer: Story = {
  args: {
    ending: {
      type: "answered",
      turns: 1,
      truncated: true,
      cut: "providerError",
    },
  },
  play: async ({ canvasElement }) => {
    await expect(
      within(canvasElement).getByText(/reported an error mid-answer/)
    ).toBeVisible()
  },
}

export const PausedByTheProvider: Story = {
  args: { ending: { type: "paused", turns: 1 } },
  play: async ({ canvasElement }) => {
    // Resuming costs tokens: it is a click, never automatic.
    await expect(
      within(canvasElement).getByRole("button", { name: "Continue" })
    ).toBeEnabled()
  },
}

export const RefusedByTheModel: Story = {
  args: { ending: { type: "refused", turns: 1 }, onContinue: undefined },
  play: async ({ canvasElement }) => {
    await expect(
      within(canvasElement).getByText(/declined to answer/)
    ).toBeVisible()
  },
}

export const TurnLimit: Story = {
  args: { ending: { type: "turnLimit", turns: 8 } },
}

export const AgentRequestLimit: Story = {
  args: { ending: { type: "agentLimit" } },
}

export const StoppedBeforeTheAgentAnswered: Story = {
  args: { ending: { type: "cancelled", turns: 0 }, onContinue: undefined },
}

export const UnknownToThisBuild: Story = {
  args: { ending: { type: "unknown" }, onContinue: undefined },
  play: async ({ canvasElement }) => {
    await expect(
      within(canvasElement).getByText(/Nothing was hidden/)
    ).toBeVisible()
  },
}

export const ContinueWaitsWhileBusy: Story = {
  args: { ending: { type: "paused", turns: 1 }, continueDisabled: true },
  play: async ({ canvasElement }) => {
    await expect(
      within(canvasElement).getByRole("button", { name: "Continue" })
    ).toBeDisabled()
  },
}
