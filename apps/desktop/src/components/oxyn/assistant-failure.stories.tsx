import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, within } from "storybook/test"

import { AssistantFailure } from "./assistant-failure"
import type { FailedEntry } from "@/features/assistant/transcript"

const base: FailedEntry = {
  kind: "failed",
  key: "f-1",
  message: "provider error: 529 overloaded_error: Overloaded",
  category: "provider",
  signIn: null,
  foundElsewhere: null,
  exit: null,
  retryable: true,
}

const meta = {
  title: "Oxyn/Assistant/Failure",
  component: AssistantFailure,
  args: {
    entry: base,
    canRetry: true,
    signInStates: {},
    onRetry: fn(),
    onSignIn: fn(),
    onCopy: fn(() => true),
  },
  decorators: [
    (Story) => (
      <div className="max-w-xl p-4">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof AssistantFailure>

export default meta
type Story = StoryObj<typeof meta>

export const ProviderFailed: Story = {
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    // The server's words, whole: the public reads error messages.
    await expect(canvas.getByText(/529 overloaded_error/)).toBeVisible()
    await userEvent.click(canvas.getByRole("button", { name: "Ask again" }))
    await expect(args.onRetry).toHaveBeenCalledOnce()
  },
}

export const RefusedByTheTier: Story = {
  args: {
    entry: {
      ...base,
      category: "refused",
      retryable: false,
      message:
        "This connection is local-only, and this provider's endpoint leaves this machine. Nothing was sent.",
    },
  },
  play: async ({ canvasElement }) => {
    // Asking again would be refused again: it is not offered.
    await expect(
      within(canvasElement).queryByRole("button", { name: /again/ })
    ).toBeNull()
  },
}

export const AgentProgramNotFound: Story = {
  args: {
    entry: {
      ...base,
      category: "agentNotFound",
      message:
        "`npx` was not found; check the agent's command in the AI settings",
      foundElsewhere: "/opt/homebrew/bin/npx",
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText("/opt/homebrew/bin/npx")).toBeVisible()
    await expect(
      canvas.getByRole("button", { name: "Try again" })
    ).toBeEnabled()
  },
}

export const AgentNeedsASignIn: Story = {
  args: {
    entry: {
      ...base,
      category: "agentSignIn",
      message: "the agent needs you to sign in before it can answer",
      signIn: {
        agent: "Claude Code",
        methods: [],
        terminalCommand: "claude auth login",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText("claude auth login")).toBeVisible()
    await expect(
      canvas.getByRole("button", { name: "Ask again" })
    ).toBeEnabled()
  },
}

export const AgentSpeaksAnotherVersion: Story = {
  args: {
    entry: {
      ...base,
      category: "agentIncompatible",
      retryable: false,
      message: "the agent speaks protocol version 2, and Oxyn speaks version 1",
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText(/protocol version 2/)).toBeVisible()
    // Oxyn pins the adapter it launches: « update it » is not the user's to do.
    await expect(canvas.queryByText(/Update the agent/)).toBeNull()
  },
}

/** Died without a word, or before it could be read: no « Details » at all. */
export const AgentStopped: Story = {
  args: {
    entry: {
      ...base,
      category: "agentExited",
      message: "the agent process stopped; asking again starts it again",
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(
      canvas.getByRole("button", { name: "Restart the agent and ask again" })
    ).toBeVisible()
    await expect(canvas.queryByRole("button", { name: "Details" })).toBeNull()
  },
}

/** The cause, folded under « Details », as the process wrote it. */
export const AgentStoppedWithItsLastWords: Story = {
  args: {
    entry: {
      ...base,
      category: "agentExited",
      message: "the agent process stopped; asking again starts it again",
      exit: {
        code: 1,
        output:
          "npm ERR! code E404\nnpm ERR! 404 Not Found - GET https://registry.npmjs.org/@agentclientprotocol%2fclaude-agent-acp\nError: API key <ANTHROPIC_API_KEY redacted> was refused",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.queryByText(/npm ERR!/)).toBeNull()
    await userEvent.click(canvas.getByRole("button", { name: "Details" }))
    await expect(
      canvas.getByText("The process exited with code 1.")
    ).toBeVisible()
    const output = canvas.getByLabelText("The agent's last output")
    await expect(output).toHaveTextContent(/404 Not Found/)
    // What Oxyn handed the process arrives already replaced by a marker.
    await expect(output).toHaveTextContent(/<ANTHROPIC_API_KEY redacted>/)
  },
}

export const AgentStartTimedOut: Story = {
  args: {
    entry: {
      ...base,
      category: "agentTimedOut",
      message:
        "Claude Code did not finish starting within 120 seconds, so Oxyn stopped it. Nothing was sent.",
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(
      canvas.getByText("The agent took too long to start")
    ).toBeVisible()
    await expect(
      canvas.getByRole("button", { name: "Restart the agent and ask again" })
    ).toBeEnabled()
  },
}

export const RetryWaitsWhileSomethingRuns: Story = {
  args: { canRetry: false },
  play: async ({ canvasElement }) => {
    await expect(
      within(canvasElement).getByRole("button", { name: "Ask again" })
    ).toBeDisabled()
  },
}
