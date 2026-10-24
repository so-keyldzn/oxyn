import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, within } from "storybook/test"

import { AssistantAnswerActions } from "./assistant-answer-actions"
import { AssistantEnding } from "./assistant-ending"
import { AssistantTurnIdentity, AssistantTurnMeta } from "./assistant-turn"

const meta = {
  title: "Oxyn/Assistant/Turn",
  component: AssistantTurnIdentity,
  args: {
    destination: {
      kind: "provider",
      label: "Work account",
      model: "claude-sonnet-5",
      reach: "remote",
      agentVersion: null,
    },
    brand: "claude",
  },
  decorators: [
    (Story) => (
      <div className="flex max-w-xl flex-col gap-3 p-4">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof AssistantTurnIdentity>

export default meta
type Story = StoryObj<typeof meta>

export const ProviderTurn: Story = {
  play: async ({ canvasElement }) => {
    await expect(canvasElement).toHaveTextContent(
      "Work account · claude-sonnet-5"
    )
  },
}

/** An agent's turn names the version it reported, not a model. */
export const AgentTurn: Story = {
  args: {
    destination: {
      kind: "agent",
      label: "Claude Code",
      model: null,
      reach: "unresolved",
      agentVersion: "Claude Agent 0.78.0",
    },
    brand: "claude-code",
  },
  play: async ({ canvasElement }) => {
    await expect(canvasElement).toHaveTextContent(
      "Claude Code · Claude Agent 0.78.0"
    )
  },
}

/**
 * A destination no longer declared keeps its recorded name and gets the
 * generic icon of its kind — never another brand's mark.
 */
export const NoLongerDeclared: Story = {
  args: {
    destination: {
      kind: "agent",
      label: "my-agent",
      model: null,
      reach: "unresolved",
      agentVersion: null,
    },
    brand: null,
  },
}

/** The quiet end of a turn: ending, usage and actions on one row. */
export const MetaRow: Story = {
  render: () => (
    <AssistantTurnMeta
      ending={
        <AssistantEnding
          ending={{ type: "answered", turns: 1, truncated: false, cut: null }}
          quiet
        />
      }
      usage={{
        input: 8200,
        output: 410,
        cacheRead: 6000,
        cacheWrite: null,
        turns: 1,
      }}
      contextWindow={null}
      cost={{ inputPerMillion: 3, outputPerMillion: 15, currency: "USD" }}
      actions={
        <AssistantAnswerActions
          text="1,204."
          canRegenerate
          onRegenerate={fn()}
          onCopy={fn(() => true)}
        />
      }
    />
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText("Answered in 1 turn.")).toBeVisible()
    await expect(canvas.getByText("8.2K in")).toBeVisible()
    // The actions are reached from the keyboard, by their names.
    await userEvent.tab()
    await expect(
      canvas.getByRole("button", { name: "Copy answer" })
    ).toHaveFocus()
    await userEvent.tab()
    await expect(
      canvas.getByRole("button", { name: "Regenerate answer" })
    ).toHaveFocus()
  },
}

/** An ending that asks for attention keeps its own line, never the meta row. */
export const NotableEndingIsNeverQuiet: Story = {
  render: () => (
    <AssistantEnding
      ending={{
        type: "answered",
        turns: 3,
        truncated: true,
        cut: "tokenLimit",
      }}
      quiet
    />
  ),
  play: async ({ canvasElement }) => {
    const ending = canvasElement.querySelector("[data-slot=assistant-ending]")
    await expect(ending?.tagName).toBe("DIV")
    await expect(ending).toHaveTextContent(
      "Answer cut short by the token limit after 3 turns. It is not finished."
    )
  },
}
