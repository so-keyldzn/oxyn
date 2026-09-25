import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

import { AssistantHeader } from "./assistant-header"
import {
  destinations,
  localProvider,
  remoteProvider,
  unresolvedProvider,
} from "./assistant-fixtures"
import { expectContainedInFrame, openFrame } from "./frame-overflow"
import type { DestinationOption } from "@/features/assistant/availability"

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

const LONG_MODEL =
  "arn:aws:bedrock:eu-west-3:123456789012:inference-profile/eu.anthropic.claude-sonnet-analytics-team-staging-v2:0"

const LONG_DESTINATIONS: Array<DestinationOption> = [
  {
    ...destinations[0]!,
    label: "Work account — the analytics team's Bedrock gateway in eu-west-3",
    model: LONG_MODEL,
  },
  {
    ...destinations[1]!,
    label: "codex_analytics_team_staging_workspace_agent_with_a_long_name",
    usable: false,
    reason:
      "Not found on PATH: /Users/analyst/.local/share/agents/codex_analytics_team_staging/bin/codex",
  },
]

/**
 * Axe runs after `play`, on whatever is still mounted: a list animating out
 * keeps its focus guards and its listbox for a few frames, and the
 * suite's pace decides whether axe sees them.
 */
async function closeList() {
  await userEvent.keyboard("{Escape}")
  await waitFor(() => {
    // Hidden once closed, but kept mounted: the accessibility tree, as axe
    // reads it, is what must be empty.
    expect(within(document.body).queryByRole("listbox")).toBeNull()
    expect(document.querySelector("[data-base-ui-focus-guard]")).toBeNull()
  })
}

/**
 * Long provider, agent and model names: the header's row keeps its selects
 * inside, and each select's list stays inside its own frame.
 */
export const LongNamesStayInTheirFrames: Story = {
  args: {
    destinations: LONG_DESTINATIONS,
    selected: LONG_DESTINATIONS[0]!,
    model: LONG_MODEL,
  },
  play: async ({ canvasElement }) => {
    const header = canvasElement.querySelector("header")
    await expect(header).not.toBeNull()
    await expectContainedInFrame(header as HTMLElement)

    const canvas = within(canvasElement)
    await userEvent.click(canvas.getByRole("combobox", { name: "Who answers" }))
    await expectContainedInFrame(await openFrame("select-content"))
    await closeList()

    await userEvent.click(canvas.getByRole("combobox", { name: "Model" }))
    await expectContainedInFrame(await openFrame("select-content"))
    await closeList()
  },
}

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
    await expect(canvas.getByText("Metadata · Agent-managed")).toBeVisible()
    await expect(canvas.queryByText(/Unresolved/)).toBeNull()
    await expect(
      canvas.getByLabelText("Privacy tier Metadata, destination Agent-managed")
    ).toBeVisible()
  },
}

/**
 * A provider whose endpoint is not resolved yet keeps « Unresolved »: the word
 * is its own, and an agent never borrows it (ExternalAgent above).
 */
export const UnresolvedProvider: Story = {
  args: {
    destinations: [
      {
        key: `provider:${unresolvedProvider.id}`,
        kind: "provider",
        id: unresolvedProvider.id,
        label: unresolvedProvider.label,
        model: unresolvedProvider.model,
        reach: "unresolved",
        usable: true,
        reason: null,
      },
    ],
    selected: {
      key: `provider:${unresolvedProvider.id}`,
      kind: "provider",
      id: unresolvedProvider.id,
      label: unresolvedProvider.label,
      model: unresolvedProvider.model,
      reach: "unresolved",
      usable: true,
      reason: null,
    },
    model: unresolvedProvider.model,
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText("Metadata · Unresolved")).toBeVisible()
    await expect(canvas.queryByText(/Agent-managed/)).toBeNull()
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
    await closeList()
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
      ignoredMentions: 1,
      omittedMentions: 2,
    },
  },
  play: async ({ canvasElement }) => {
    // What was left out is counted, and said.
    const canvas = within(canvasElement)
    await expect(canvas.getByText(/3 omitted to fit/)).toBeVisible()
    await expect(
      canvas.getByText(/1 row sample\(s\) withheld by the tier/)
    ).toBeVisible()
    // A mention the user typed and that did not go is never silent.
    await expect(
      canvas.getByText(/1 mentioned object\(s\) not found/)
    ).toBeVisible()
    await expect(
      canvas.getByText(/2 mentioned object\(s\) named only/)
    ).toBeVisible()
  },
}
