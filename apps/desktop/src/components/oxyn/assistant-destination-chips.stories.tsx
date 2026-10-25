import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

import {
  AssistantDestinationChips,
  AssistantModelListNote,
} from "./assistant-destination-chips"
import {
  destinations,
  localProvider,
  remoteProvider,
} from "./assistant-fixtures"
import { expectContainedInFrame, openFrame } from "./frame-overflow"
import { restrictedToAgent } from "@/features/assistant/agent-options"
import type { DestinationOption } from "@/features/assistant/availability"

const meta = {
  title: "Oxyn/Assistant/DestinationChips",
  component: AssistantDestinationChips,
  args: {
    destinations,
    selected: destinations[0] ?? null,
    model: remoteProvider.model,
    models: null,
    running: false,
    onSelectDestination: fn(),
    onSelectModel: fn(),
    onModelsWanted: fn(),
    efforts: ["low", "medium", "high"],
    effort: null,
    onSelectEffort: fn(),
  },
  decorators: [
    // The composer's bottom bar: the container the chips narrow with.
    (Story) => (
      <div className="@container/composer flex w-96 min-w-0 items-center rounded-xl border p-1.5">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof AssistantDestinationChips>

export default meta
type Story = StoryObj<typeof meta>

const body = () => within(document.body)

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
    expect(body().queryByRole("listbox")).toBeNull()
    expect(document.querySelector("[data-base-ui-focus-guard]")).toBeNull()
  })
}

const localOption: DestinationOption = {
  key: `provider:${localProvider.id}`,
  kind: "provider",
  id: localProvider.id,
  label: localProvider.label,
  model: localProvider.model,
  reach: "local",
  usable: true,
  reason: null,
  brand: localProvider.kind,
}

/**
 * A provider reads as its mark then its model, the effort beside them: three
 * named controls, each opening its own list from the keyboard.
 */
export const ProviderWithItsModel: Story = {
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    const who = canvas.getByRole("combobox", { name: "Who answers" })
    const model = canvas.getByRole("combobox", { name: "Model" })
    await expect(model).toHaveTextContent(remoteProvider.model)
    await expect(
      canvas.getByRole("combobox", { name: "Reasoning effort: Default" })
    ).toBeVisible()
    who.focus()
    await userEvent.keyboard("{Enter}")
    // Awaited: the list fades in.
    const list = await body().findByRole("listbox", { name: "Who answers" })
    await waitFor(() => expect(list).toBeVisible())
    await closeList()
    model.focus()
    await userEvent.keyboard("{Enter}")
    await expect(args.onModelsWanted).toHaveBeenCalled()
    await closeList()
  },
}

/** Chips inert while an answer is written, each with its own reason. */
export const WhileAnswering: Story = {
  args: { running: true },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(
      canvas.getByRole("combobox", { name: "Who answers" })
    ).toBeDisabled()
    await expect(canvas.getByRole("combobox", { name: "Model" })).toBeDisabled()
  },
}

/** A model that declares no effort has no effort chip: nothing greyed. */
export const NoEffortDeclared: Story = {
  args: { efforts: [] },
  play: async ({ canvasElement }) => {
    await expect(
      within(canvasElement).queryByRole("combobox", {
        name: /Reasoning effort/,
      })
    ).toBeNull()
  },
}

/**
 * An agent reads as its mark and its name; its own `thoughtLevel` plays the
 * effort's part, so Oxyn's effort chip is absent.
 */
export const ExternalAgent: Story = {
  args: { selected: destinations[1] ?? null, model: null },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(
      canvas.getByRole("combobox", { name: "Who answers" })
    ).toHaveTextContent("Claude Code")
    await expect(canvas.queryByRole("combobox", { name: "Model" })).toBeNull()
    await expect(
      canvas.queryByRole("combobox", { name: /Reasoning effort/ })
    ).toBeNull()
  },
}

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
 * Long provider, agent and model names in a narrow composer: the chips cut
 * with an ellipsis and never wrap, and each list stays inside its own frame.
 */
export const LongNamesStayInTheirFrames: Story = {
  args: {
    destinations: LONG_DESTINATIONS,
    selected: LONG_DESTINATIONS[0]!,
    model: LONG_MODEL,
  },
  decorators: [
    (Story) => (
      <div className="w-[300px]">
        <Story />
      </div>
    ),
  ],
  play: async ({ canvasElement }) => {
    await expectContainedInFrame(canvasElement.firstElementChild as HTMLElement)
    const canvas = within(canvasElement)
    await userEvent.click(canvas.getByRole("combobox", { name: "Who answers" }))
    await expectContainedInFrame(await openFrame("select-content"))
    await closeList()

    await userEvent.click(canvas.getByRole("combobox", { name: "Model" }))
    await expectContainedInFrame(await openFrame("select-content"))
    await closeList()
  },
}

/** Under `Local`, an agent is closed — and the list says why, on it. */
export const AgentClosedOnALocalConnection: Story = {
  args: {
    destinations: [
      localOption,
      {
        ...(destinations[1] ?? destinations[0]!),
        usable: false,
        reason:
          "This connection is local-only, and Oxyn cannot see where an external agent sends data.",
      },
    ],
    selected: localOption,
    model: localProvider.model,
  },
  play: async ({ canvasElement }) => {
    await userEvent.click(
      within(canvasElement).getByRole("combobox", { name: "Who answers" })
    )
    const agent = await body().findByRole("option", { name: /Claude Code/ })
    await expect(agent).toHaveAttribute("aria-disabled", "true")
    await expect(agent).toHaveTextContent(
      /cannot see where an external agent sends data/
    )
    await closeList()
  },
}

/**
 * The conversation runs an agent written for local models only: the hosted
 * provider stays listed, disabled, and says why and what to do instead
 * (ADR-0049 § 6). Choosing another destination never switches the agent.
 */
export const DestinationTheAgentIsNotWrittenFor: Story = {
  args: (() => {
    const restricted = restrictedToAgent([localOption, destinations[0]!], {
      id: "0199a3c0-0000-7000-8000-0000000000a2",
      name: "Small model",
      description: "",
      origin: "user",
      error: null,
      disabledDestinations: [{ kind: "provider", id: remoteProvider.id }],
    })
    return {
      destinations: restricted,
      selected: restricted[0] ?? null,
      model: localProvider.model,
    }
  })(),
  play: async ({ canvasElement, args }) => {
    await userEvent.click(
      within(canvasElement).getByRole("combobox", { name: "Who answers" })
    )
    const hosted = await body().findByRole("option", {
      name: new RegExp(remoteProvider.label),
    })
    await expect(hosted).toHaveAttribute("aria-disabled", "true")
    await expect(hosted).toHaveTextContent(
      "The Small model agent is not written for it."
    )
    await expect(args.onSelectDestination).not.toHaveBeenCalled()
    await closeList()
  },
}

export const ModelsLoading: Story = {
  render: () => <AssistantModelListNote modelList={{ status: "loading" }} />,
  play: async ({ canvasElement }) => {
    await expect(
      within(canvasElement).getByText("Listing models…")
    ).toBeVisible()
  },
}

/** The endpoint's own words: the public reads error messages. */
export const ModelsFailed: Story = {
  render: () => (
    <AssistantModelListNote
      modelList={{
        status: "error",
        message: "GET /v1/models answered 401: invalid x-api-key",
      }}
    />
  ),
  play: async ({ canvasElement }) => {
    await expect(within(canvasElement).getByRole("alert")).toHaveTextContent(
      "Models could not be listed: GET /v1/models answered 401: invalid x-api-key"
    )
  },
}

export const Light: Story = { globals: { theme: "light" } }
