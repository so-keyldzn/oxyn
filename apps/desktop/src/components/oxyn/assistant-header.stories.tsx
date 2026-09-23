import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

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
  play: async ({ canvasElement }) => {
    // A known agent is confined: no warning.
    await expect(
      canvasElement.querySelector("[data-slot=agent-unconfined]")
    ).toBeNull()
    // Its destination is unknowable, not « unresolved »: said as what it is.
    const canvas = within(canvasElement)
    await expect(canvas.getByText("Metadata · External agent")).toBeVisible()
    await expect(canvas.queryByText(/Unresolved/)).toBeNull()
  },
}

const startupControls = {
  signInStates: {},
  onCancel: fn(),
  onStart: fn(),
  onSignIn: fn(),
  onCopy: fn(() => true),
}

/** The panel opened on an agent: it starts before anything is asked. */
export const AgentStarting: Story = {
  args: {
    selected: destinations[1] ?? null,
    model: null,
    agentStartup: {
      ...startupControls,
      startup: {
        status: "starting",
        key: "k",
        label: "Claude Code",
        shown: true,
      },
    },
  },
  play: async ({ canvasElement }) => {
    await expect(
      within(canvasElement).getByText("Starting Claude Code…")
    ).toBeVisible()
  },
}

/** Started ahead of the question: what it said it is, before any answer. */
export const AgentStartedBeforeAQuestion: Story = {
  args: {
    selected: destinations[1] ?? null,
    model: null,
    agentStartup: {
      ...startupControls,
      startup: {
        status: "ready",
        key: "k",
        label: "Claude Code",
        version: "Claude Agent 0.78.0",
        settings: { modes: [], currentMode: null, options: [] },
      },
    },
  },
  play: async ({ canvasElement }) => {
    await expect(
      within(canvasElement).getByText(/Claude Agent 0\.78\.0 ·/)
    ).toBeVisible()
  },
}

/** While a question runs, its own Stop counts: no second « Cancel ». */
export const AgentStartHiddenWhileAnswering: Story = {
  args: {
    ...AgentStarting.args,
    running: true,
  },
  play: async ({ canvasElement }) => {
    await expect(
      within(canvasElement).queryByText("Starting Claude Code…")
    ).toBeNull()
  },
}

/** Under `Local`, an agent is closed — and the list says why, on it. */
export const AgentClosedOnALocalConnection: Story = {
  args: {
    tier: "local",
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
      {
        ...(destinations[1] ?? destinations[0]!),
        usable: false,
        reason:
          "This connection is local-only, and Oxyn cannot see where an external agent sends data.",
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
  play: async ({ canvasElement }) => {
    await userEvent.click(
      within(canvasElement).getByRole("combobox", { name: "Who answers" })
    )
    const agent = await within(document.body).findByRole("option", {
      name: /Claude Code/,
    })
    await expect(agent).toHaveAttribute("aria-disabled", "true")
    await expect(agent).toHaveTextContent(
      /cannot see where an external agent sends data/
    )
    await userEvent.keyboard("{Escape}")
    // Axe runs after `play`: wait for the popup and its focus guards to go.
    await waitFor(() =>
      expect(document.querySelector("[data-base-ui-focus-guard]")).toBeNull()
    )
  },
}

export const ModelsLoading: Story = {
  args: { modelList: { status: "loading" } },
  play: async ({ canvasElement }) => {
    await expect(
      within(canvasElement).getByText("Listing models…")
    ).toBeVisible()
  },
}

/** The endpoint's own words: the public reads error messages. */
export const ModelsFailed: Story = {
  args: {
    modelList: {
      status: "error",
      message: "GET /v1/models answered 401: invalid x-api-key",
    },
  },
  play: async ({ canvasElement }) => {
    await expect(within(canvasElement).getByRole("alert")).toHaveTextContent(
      "Models could not be listed: GET /v1/models answered 401: invalid x-api-key"
    )
  },
}

export const UnknownExternalAgent: Story = {
  args: {
    selected: destinations[1]
      ? { ...destinations[1], label: "my-agent", unconfined: true }
      : null,
    model: null,
  },
  play: async ({ canvasElement }) => {
    // Declared by hand: Oxyn says what it cannot guarantee (ADR-0032).
    await expect(
      within(canvasElement).getByText(/cannot stop it from running commands/)
    ).toBeVisible()
  },
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
