import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, within } from "storybook/test"

import {
  AssistantAgentTool,
  AssistantMemoryReset,
  AssistantPermissionRefused,
  AssistantWaiting,
} from "./assistant-agent-activity"

const meta = {
  title: "Oxyn/Assistant/AgentActivity",
  component: AssistantAgentTool,
  decorators: [
    (Story) => (
      <div className="flex max-w-xl flex-col gap-3 p-4">
        <Story />
      </div>
    ),
  ],
  args: { tool: "read", status: "running" },
} satisfies Meta<typeof AssistantAgentTool>

export default meta
type Story = StoryObj<typeof meta>

export const AgentToolRunning: Story = {}

export const AgentToolFinished: Story = {
  args: { tool: "think", status: "completed" },
}

export const AgentToolWithoutAKind: Story = {
  args: { tool: null, status: "failed" },
  play: async ({ canvasElement }) => {
    await expect(
      within(canvasElement).getByText("Agent step · tool · failed")
    ).toBeVisible()
  },
}

export const MachineActionRefused: Story = {
  render: () => (
    <AssistantPermissionRefused
      action="execute"
      reason="Oxyn does not run commands on behalf of an agent. Database work goes through Oxyn's own tools, which are reviewed."
    />
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText(/Oxyn\s+refused/)).toBeVisible()
    // ADR-0026: nothing here can grant it.
    await expect(canvas.queryByRole("button")).toBeNull()
  },
}

export const MemoryResetByTheTier: Story = {
  render: () => <AssistantMemoryReset reason="tierChanged" />,
  play: async ({ canvasElement }) => {
    await expect(
      within(canvasElement).getByText(/privacy tier changed/)
    ).toBeVisible()
  },
}

export const MemoryResetByARestartedAgent: Story = {
  render: () => <AssistantMemoryReset reason="agentRestarted" />,
}

export const Waiting: Story = {
  render: () => <AssistantWaiting label="Waiting for the model…" />,
}
