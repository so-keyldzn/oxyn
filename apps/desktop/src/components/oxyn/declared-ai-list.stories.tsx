import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, within } from "storybook/test"

import { DeclaredAgentItem, DeclaredProviderItem } from "./declared-ai-list"
import { externalAgent, remoteProvider } from "./assistant-fixtures"

const meta = {
  title: "Oxyn/Settings/DeclaredProviderItem",
  component: DeclaredProviderItem,
  args: {
    provider: remoteProvider,
    listed: undefined,
    busy: false,
    onListModels: fn(),
    onEdit: fn(),
    onRemove: fn(),
  },
  decorators: [
    (Story) => (
      <div className="max-w-2xl p-4">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof DeclaredProviderItem>

export default meta
type Story = StoryObj<typeof meta>

export const Declared: Story = {
  play: async ({ canvasElement, args }) => {
    await userEvent.click(
      within(canvasElement).getByRole("button", { name: "List models" })
    )
    await expect(args.onListModels).toHaveBeenCalled()
  },
}

export const ListingModels: Story = {
  args: { listed: { status: "loading" } },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText("Asking the endpoint…")).toBeVisible()
    await expect(
      canvas.getByRole("button", { name: "List models" })
    ).toBeDisabled()
  },
}

export const OneModelListed: Story = {
  args: {
    listed: {
      status: "ready",
      models: [
        {
          id: "claude-sonnet-5",
          displayName: "Claude Sonnet 5",
          contextWindow: null,
          cost: null,
          reasoningEfforts: [],
        },
      ],
    },
  },
  play: async ({ canvasElement }) => {
    await expect(
      within(canvasElement).getByText(
        "The endpoint answered · 1 model: claude-sonnet-5"
      )
    ).toBeVisible()
  },
}

export const NoModelListed: Story = {
  args: { listed: { status: "ready", models: [] } },
}

export const ListingFailed: Story = {
  args: {
    listed: {
      status: "error",
      message: "error sending request for url (https://api.anthropic.com)",
    },
  },
}

export const Agent: Story = {
  render: () => (
    <DeclaredAgentItem
      agent={externalAgent}
      busy={false}
      onReplace={fn()}
      onRemove={fn()}
    />
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText(/2 arguments/)).toBeVisible()
    await expect(
      canvas.getByRole("button", { name: `Replace… ${externalAgent.label}` })
    ).toBeEnabled()
  },
}

export const UnconfinedAgent: Story = {
  render: () => (
    <DeclaredAgentItem
      agent={{
        ...externalAgent,
        preset: null,
        confined: false,
        argCount: 1,
      }}
      busy={false}
      onReplace={fn()}
      onRemove={fn()}
    />
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText(/1 argument$/)).toBeVisible()
    await expect(canvas.getByText(/Not a known agent/)).toBeVisible()
  },
}

export const KnownAgentNotConfined: Story = {
  render: () => (
    <DeclaredAgentItem
      agent={{ ...externalAgent, preset: "codex", confined: false }}
      busy={false}
      onReplace={fn()}
      onRemove={fn()}
    />
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    // Known, yet not confined: never called unknown, and never « Restricted ».
    await expect(canvas.queryByText(/Not a known agent/)).toBeNull()
    await expect(canvas.queryByText("Restricted by Oxyn")).toBeNull()
    await expect(
      canvas.getByText(/your organization's configuration turns on/)
    ).toBeVisible()
  },
}
