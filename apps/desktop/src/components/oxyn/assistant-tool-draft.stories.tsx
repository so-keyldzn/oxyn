import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, within } from "storybook/test"

import { AssistantToolDraft } from "./assistant-tool-draft"

const meta = {
  title: "Oxyn/Assistant/ToolDraft",
  component: AssistantToolDraft,
  args: {
    entry: {
      kind: "toolDraft",
      key: "d-0",
      index: 0,
      tool: "execute_query",
      arguments: '{"sql":"SELECT count(*) FROM public.clie',
    },
  },
  decorators: [
    (Story) => (
      <div className="max-w-xl p-4">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof AssistantToolDraft>

export default meta
type Story = StoryObj<typeof meta>

export const Writing: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText(/nothing has run/)).toBeVisible()
    await expect(
      canvas.getByText(/SELECT count\(\*\) FROM public.clie/)
    ).toBeVisible()
  },
}

export const JustAnnounced: Story = {
  args: {
    entry: {
      kind: "toolDraft",
      key: "d-1",
      index: 1,
      tool: "refresh_catalog",
      arguments: "",
    },
  },
}
