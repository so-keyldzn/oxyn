import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, within } from "storybook/test"

import { AssistantVersions } from "./assistant-versions"

const meta = {
  title: "Oxyn/Assistant/Versions",
  component: AssistantVersions,
  args: {
    versions: { position: 2, count: 3, previous: 4, next: 6 },
    onSelect: fn(),
  },
  decorators: [
    (Story) => (
      <div className="p-4">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof AssistantVersions>

export default meta
type Story = StoryObj<typeof meta>

export const Middle: Story = {
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText("2 / 3")).toBeVisible()
    await userEvent.click(
      canvas.getByRole("button", { name: "Previous version" })
    )
    await expect(args.onSelect).toHaveBeenCalledWith(4)
    await userEvent.click(canvas.getByRole("button", { name: "Next version" }))
    await expect(args.onSelect).toHaveBeenCalledWith(6)
  },
}

export const First: Story = {
  args: { versions: { position: 1, count: 3, previous: null, next: 6 } },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(
      canvas.getByRole("button", { name: "Previous version" })
    ).toBeDisabled()
  },
}

export const Last: Story = {
  args: { versions: { position: 3, count: 3, previous: 4, next: null } },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(
      canvas.getByRole("button", { name: "Next version" })
    ).toBeDisabled()
  },
}

export const OnlyOneVersion: Story = {
  args: { versions: { position: 1, count: 1, previous: null, next: null } },
  play: async ({ canvasElement }) => {
    // Nothing to navigate: no control at all rather than two dead arrows.
    await expect(canvasElement.textContent).toBe("")
  },
}

export const WhileRunning: Story = {
  args: { disabled: true },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    // Switching version mid-run would show an answer the stream is not writing.
    await expect(
      canvas.getByRole("button", { name: "Previous version" })
    ).toBeDisabled()
    await expect(
      canvas.getByRole("button", { name: "Next version" })
    ).toBeDisabled()
  },
}

export const ManyVersions: Story = {
  args: { versions: { position: 12, count: 128, previous: 4, next: 6 } },
}
