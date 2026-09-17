import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, within } from "storybook/test"

import { AssistantUsage } from "./assistant-usage"

const meta = {
  title: "Oxyn/Assistant/Usage",
  component: AssistantUsage,
  args: {
    usage: {
      input: 12_400,
      output: 820,
      cacheRead: 9_200,
      cacheWrite: null,
      turns: 2,
    },
    contextWindow: null,
    cost: { inputPerMillion: 3, outputPerMillion: 15, currency: "USD" },
  },
  decorators: [
    (Story) => (
      <div className="max-w-xl p-4">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof AssistantUsage>

export default meta
type Story = StoryObj<typeof meta>

export const WithAPublishedPrice: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText("12.4K in")).toBeVisible()
    await expect(canvas.getByText("9.2K cache read")).toBeVisible()
    // What nobody declared is left out, not shown as zero.
    await expect(canvas.queryByText(/cache write/)).toBeNull()
    await expect(canvas.getByText(/≈/)).toBeVisible()
  },
}

export const WithoutAPublishedPrice: Story = {
  args: { cost: null },
  play: async ({ canvasElement }) => {
    // No price is written in Oxyn (I-12): none is shown.
    await expect(within(canvasElement).queryByText(/≈/)).toBeNull()
  },
}

export const AnAgentsContextWindow: Story = {
  args: {
    usage: null,
    cost: null,
    contextWindow: {
      used: 48_000,
      size: 200_000,
      cost: { amount: 0.42, currency: "USD" },
    },
  },
  play: async ({ canvasElement }) => {
    await expect(
      within(canvasElement).getByText(/context 48K \/ 200K/)
    ).toBeVisible()
  },
}

export const NothingDeclared: Story = {
  args: { usage: null, contextWindow: null, cost: null },
  play: async ({ canvasElement }) => {
    await expect(canvasElement.textContent).toBe("")
  },
}

export const ACurrencyTheRuntimeIgnores: Story = {
  args: {
    usage: {
      input: 10,
      output: 10,
      cacheRead: null,
      cacheWrite: null,
      turns: 1,
    },
    contextWindow: null,
    cost: { inputPerMillion: 1, outputPerMillion: 1, currency: "XYZ" },
  },
}
