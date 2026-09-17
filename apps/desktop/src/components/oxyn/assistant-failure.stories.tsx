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
    await expect(
      within(canvasElement).getByText(/Update the agent or its adapter/)
    ).toBeVisible()
  },
}

export const AgentStopped: Story = {
  args: {
    entry: {
      ...base,
      category: "agentExited",
      message:
        "the agent process stopped; start the conversation again to relaunch it",
    },
  },
  play: async ({ canvasElement }) => {
    await expect(
      within(canvasElement).getByRole("button", { name: "Restart the agent" })
    ).toBeVisible()
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
