import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

import { AssistantHeader } from "./assistant-header"
import {
  destinations,
  localProvider,
  remoteProvider,
  unresolvedProvider,
} from "./assistant-fixtures"
import { expectContainedInFrame } from "./frame-overflow"

const meta = {
  title: "Oxyn/Assistant/Header",
  component: AssistantHeader,
  args: {
    connectionName: "commerce-prod",
    environment: "production",
    tier: "metadata",
    selected: destinations[0] ?? null,
    model: remoteProvider.model,
    running: false,
    context: null,
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

/**
 * Long connection, provider and model names: the identity row and the privacy
 * line stay inside the header, the names cut with an ellipsis — never the
 * environment badge nor the tier. The selectors' own lists are held in
 * `AssistantDestinationChips`' stories.
 */
export const LongNamesStayInTheirFrames: Story = {
  args: {
    connectionName:
      "commerce-production-replica-eu-west-3-analytics-readonly-pool",
    selected: {
      ...destinations[0]!,
      label: "Work account — the analytics team's Bedrock gateway in eu-west-3",
      model: LONG_MODEL,
    },
    model: LONG_MODEL,
  },
  decorators: [
    (Story) => (
      <div className="w-[320px] border">
        <Story />
      </div>
    ),
  ],
  play: async ({ canvasElement }) => {
    const header = canvasElement.querySelector("header")
    await expect(header).not.toBeNull()
    await expectContainedInFrame(header as HTMLElement)
    const canvas = within(canvasElement)
    await expect(canvas.getByText("PRODUCTION")).toBeVisible()
    await expect(canvas.getByText("Metadata · Cloud")).toBeVisible()
  },
}

export const MetadataToCloud: Story = {
  play: async ({ canvasElement }) => {
    // The tier and where it goes, in words, before anything is asked.
    const canvas = within(canvasElement)
    await expect(canvas.getByText("Metadata · Cloud")).toBeVisible()
    await expect(canvasElement.textContent).not.toContain(remoteProvider.id)
    // Who answers, in the header line, not only in the composer's chip.
    const who = canvasElement.querySelector("[data-slot=assistant-who-answers]")
    await expect(who).toHaveTextContent(
      `Who answers: ${remoteProvider.label} · ${remoteProvider.model}`
    )
  },
}

/** The tier's sentence is the badge's description, one focus away. */
export const TierSentenceOnFocus: Story = {
  play: async ({ canvasElement }) => {
    const badge = within(canvasElement).getByLabelText(
      "Privacy tier Metadata, destination Cloud"
    )
    await userEvent.tab()
    await expect(badge).toHaveFocus()
    // Awaited: the tooltip fades in.
    const sentence = await within(document.body).findByText(
      /Metadata is not « nothing leaves »/
    )
    await waitFor(() => expect(sentence).toBeVisible())
    await userEvent.keyboard("{Escape}")
    await waitFor(() =>
      expect(
        within(document.body).queryByText(/Metadata is not « nothing leaves »/)
      ).toBeNull()
    )
  },
}

export const LocalOnlyConnection: Story = {
  args: {
    tier: "local",
    environment: "development",
    connectionName: "analytics-dev",
    selected: {
      key: `provider:${localProvider.id}`,
      kind: "provider",
      id: localProvider.id,
      label: localProvider.label,
      model: localProvider.model,
      reach: "local",
      usable: true,
      reason: null,
      brand: localProvider.kind,
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
    // The permanent mention of a chosen agent (docs/UX-SPEC.md).
    await expect(
      canvas.getByText(/Oxyn cannot see where it sends your question/)
    ).toBeVisible()
  },
}

/**
 * A provider whose endpoint is not resolved yet keeps « Unresolved »: the word
 * is its own, and an agent never borrows it (ExternalAgent above).
 */
export const UnresolvedProvider: Story = {
  args: {
    selected: {
      key: `provider:${unresolvedProvider.id}`,
      kind: "provider",
      id: unresolvedProvider.id,
      label: unresolvedProvider.label,
      model: unresolvedProvider.model,
      reach: "unresolved",
      usable: true,
      reason: null,
      brand: unresolvedProvider.kind,
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
      canvasElement.querySelector("[data-slot=assistant-who-answers]")
    ).toHaveTextContent("Claude Code · Claude Agent 0.78.0")
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

/** A destination the tier refuses says why, under who answers. */
export const SelectedDestinationRefused: Story = {
  args: {
    tier: "local",
    selected: {
      ...destinations[1]!,
      usable: false,
      reason:
        "This connection is local-only, and Oxyn cannot see where an external agent sends data.",
    },
    model: null,
  },
  play: async ({ canvasElement }) => {
    await expect(
      within(canvasElement).getByText(
        /cannot see where an external agent sends data/
      )
    ).toBeVisible()
  },
}

export const UnknownExternalAgent: Story = {
  args: {
    selected: destinations[1]
      ? {
          ...destinations[1],
          label: "my-agent",
          unconfined: true,
          brand: undefined,
        }
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

export const Light: Story = {
  args: { selected: destinations[1] ?? null, model: null },
  globals: { theme: "light" },
}
