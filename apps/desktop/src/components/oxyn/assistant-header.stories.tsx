import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, within } from "storybook/test"

import { AssistantHeader } from "./assistant-header"
import {
  destinations,
  localProvider,
  remoteProvider,
} from "./assistant-fixtures"

const meta = {
  title: "Oxyn/Assistant/Header",
  component: AssistantHeader,
  args: {
    connectionName: "commerce-prod",
    environment: "production",
    tier: "metadata",
    destinations,
    selected: destinations[0] ?? null,
    model: remoteProvider.model,
    models: null,
    running: false,
    context: null,
    onSelectDestination: fn(),
    onSelectModel: fn(),
    onModelsWanted: fn(),
  },
  decorators: [
    (Story) => (
      <div className="max-w-xl border">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof AssistantHeader>

export default meta
type Story = StoryObj<typeof meta>

export const MetadataToCloud: Story = {
  play: async ({ canvasElement }) => {
    // The tier and where it goes, in words, before anything is asked.
    await expect(
      within(canvasElement).getByText("Metadata · Cloud")
    ).toBeVisible()
    await expect(canvasElement.textContent).not.toContain(remoteProvider.id)
  },
}

export const LocalOnlyConnection: Story = {
  args: {
    tier: "local",
    environment: "development",
    connectionName: "analytics-dev",
    destinations: [
      {
        key: `provider:${localProvider.id}`,
        kind: "provider",
        id: localProvider.id,
        label: localProvider.label,
        model: localProvider.model,
        reach: "local",
        usable: true,
        reason: null,
      },
    ],
    selected: {
      key: `provider:${localProvider.id}`,
      kind: "provider",
      id: localProvider.id,
      label: localProvider.label,
      model: localProvider.model,
      reach: "local",
      usable: true,
      reason: null,
    },
    model: localProvider.model,
  },
}

export const ExternalAgent: Story = {
  args: { selected: destinations[1] ?? null, model: null },
}

export const AfterAQuestion: Story = {
  args: {
    context: {
      relations: 42,
      omittedRelations: 3,
      droppedSamples: 1,
      estimatedTokens: 5_800,
    },
  },
  play: async ({ canvasElement }) => {
    // What was left out is counted, and said.
    const canvas = within(canvasElement)
    await expect(canvas.getByText(/3 omitted to fit/)).toBeVisible()
    await expect(
      canvas.getByText(/1 row sample\(s\) withheld by the tier/)
    ).toBeVisible()
  },
}
