import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, within } from "storybook/test"

import {
  AssistantAgentTool,
  AssistantCatalogRead,
  AssistantMemoryReset,
  AssistantPermissionRefused,
  AssistantWaiting,
} from "./assistant-agent-activity"
import type { CatalogEntry } from "@/features/assistant/transcript"

const CATALOG: CatalogEntry = {
  kind: "catalog",
  key: "catalog-0",
  reading: false,
  listed: 2,
  described: 11,
  failed: 0,
  notLoaded: 0,
  unlisted: 0,
  stopped: null,
}

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
    // Read whole: no label replaces the content and drops the reason.
    const note = canvas.getByRole("note")
    await expect(note).not.toHaveAttribute("aria-label")
    await expect(note).toHaveTextContent(/does not run commands/)
  },
}

export const StepsAreNotLiveRegions: Story = {
  render: () => (
    <>
      <AssistantAgentTool tool="read" status="running" />
      <AssistantWaiting label="Waiting for the model…" />
    </>
  ),
  play: async ({ canvasElement }) => {
    // One announcement per panel state, not one per step (AssistantView).
    await expect(within(canvasElement).queryByRole("status")).toBeNull()
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

export const CatalogReading: Story = {
  render: () => <AssistantCatalogRead entry={{ ...CATALOG, reading: true }} />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText(/Reading the catalog/)).toBeVisible()
    await expect(canvas.getByText(/no row is read/)).toBeVisible()
    // A step, not an announcement (StepsAreNotLiveRegions).
    await expect(canvas.queryByRole("status")).toBeNull()
  },
}

export const CatalogRead: Story = {
  render: () => <AssistantCatalogRead entry={CATALOG} />,
  play: async ({ canvasElement }) => {
    await expect(
      within(canvasElement).getByText(/11 objects described/)
    ).toBeVisible()
  },
}

export const CatalogPartlyRead: Story = {
  render: () => (
    <AssistantCatalogRead
      entry={{
        ...CATALOG,
        described: 24,
        listed: 32,
        failed: 1,
        notLoaded: 3,
        unlisted: 40,
        stopped: "deadline",
      }}
    />
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    // What is missing is said, never left to look like the whole schema.
    await expect(canvas.getByText(/time bound/)).toBeVisible()
    await expect(canvas.getByText(/40 schemas not listed/)).toBeVisible()
    await expect(canvas.getByText(/The assistant was told/)).toBeVisible()
  },
}
