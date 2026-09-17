import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, within } from "storybook/test"

import { AssistantSignIn } from "./assistant-sign-in"

const meta = {
  title: "Oxyn/Assistant/SignIn",
  component: AssistantSignIn,
  args: {
    help: {
      agent: "Codex",
      methods: [
        {
          id: "chat-gpt",
          name: "ChatGPT",
          description: "Opens your browser. Oxyn sees no token.",
          kind: "agent",
          command: null,
        },
      ],
      terminalCommand: "codex login",
    },
    states: {},
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
} satisfies Meta<typeof AssistantSignIn>

export default meta
type Story = StoryObj<typeof meta>

export const TheAgentSignsItsUserIn: Story = {
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await userEvent.click(
      canvas.getByRole("button", { name: "Sign in with ChatGPT" })
    )
    await expect(args.onSignIn).toHaveBeenCalledWith("chat-gpt")
    await expect(
      canvas.getByText("Oxyn never sees nor keeps the agent's credentials.")
    ).toBeVisible()
  },
}

export const SigningIn: Story = {
  args: { states: { "chat-gpt": { status: "pending" } } },
  play: async ({ canvasElement }) => {
    await expect(
      within(canvasElement).getByRole("button", {
        name: /Sign in with ChatGPT/,
      })
    ).toBeDisabled()
  },
}

export const SignedIn: Story = {
  args: { states: { "chat-gpt": { status: "done" } } },
}

export const SignInFailed: Story = {
  args: {
    states: {
      "chat-gpt": { status: "error", message: "the browser did not answer" },
    },
  },
  play: async ({ canvasElement }) => {
    await expect(within(canvasElement).getByRole("alert")).toHaveTextContent(
      "the browser did not answer"
    )
  },
}

export const InATerminalOnly: Story = {
  args: {
    help: {
      agent: "Claude Code",
      methods: [
        {
          id: "claude-ai-login",
          name: "Claude Subscription",
          description: null,
          kind: "terminal",
          command:
            "npx -y @agentclientprotocol/claude-agent-acp@0.78.0 --cli auth login --claudeai",
        },
      ],
      terminalCommand: "claude auth login",
    },
  },
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    // Oxyn shows the command; it does not run a terminal.
    await expect(canvas.queryByRole("button", { name: /^Sign in/ })).toBeNull()
    await userEvent.click(canvas.getByRole("button", { name: "Copy command" }))
    await expect(args.onCopy).toHaveBeenCalledWith(
      expect.stringContaining("--cli auth login --claudeai")
    )
  },
}

export const NothingOffered: Story = {
  args: { help: { agent: "Some agent", methods: [], terminalCommand: null } },
  play: async ({ canvasElement }) => {
    await expect(
      within(canvasElement).getByText(/gave no way to sign in from here/)
    ).toBeVisible()
  },
}
