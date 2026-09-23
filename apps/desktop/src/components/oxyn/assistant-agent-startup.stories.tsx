import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, within } from "storybook/test"

import { AssistantAgentStartup } from "./assistant-agent-startup"
import type { AgentStartup } from "@/features/assistant/agent-startup"

const KEY = "agent-0badf00d|session|"

const starting: AgentStartup = {
  status: "starting",
  key: KEY,
  label: "Claude Code",
  shown: true,
}

const meta = {
  title: "Oxyn/Assistant/Agent startup",
  component: AssistantAgentStartup,
  args: {
    startup: starting,
    signInStates: {},
    onCancel: fn(),
    onStart: fn(),
    onSignIn: fn(),
    onCopy: fn(() => true),
  },
  decorators: [
    (Story) => (
      <div className="max-w-md p-3">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof AssistantAgentStartup>

export default meta
type Story = StoryObj<typeof meta>

/** The first `npx` can take tens of seconds: said, and stoppable. */
export const Starting: Story = {
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByRole("status")).toHaveTextContent(
      "Starting Claude Code…"
    )
    await userEvent.click(canvas.getByRole("button", { name: "Cancel" }))
    await expect(args.onCancel).toHaveBeenCalledOnce()
  },
}

/** A start that answers at once never flashes « Starting ». */
export const StartingNotYetShown: Story = {
  args: { startup: { ...starting, shown: false } },
  play: async ({ canvasElement }) => {
    await expect(canvasElement.textContent).toBe("")
  },
}

/** Ready: nothing here — the settings and the version speak for it. */
export const Ready: Story = {
  args: {
    startup: {
      status: "ready",
      key: KEY,
      label: "Claude Code",
      version: "Claude Agent 0.78.0",
      settings: { modes: [], currentMode: null, options: [] },
    },
  },
  play: async ({ canvasElement }) => {
    await expect(canvasElement.textContent).toBe("")
  },
}

/** Nothing started: an idle panel draws nothing. */
export const Idle: Story = {
  args: { startup: { status: "idle" } },
  play: async ({ canvasElement }) => {
    await expect(canvasElement.textContent).toBe("")
  },
}

export const StoppedByTheUser: Story = {
  args: { startup: { status: "stopped", key: KEY, label: "Claude Code" } },
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await expect(
      canvas.getByText(/Your first question starts it/)
    ).toBeVisible()
    await userEvent.click(canvas.getByRole("button", { name: "Start now" }))
    await expect(args.onStart).toHaveBeenCalledOnce()
  },
}

export const TimedOut: Story = {
  args: {
    startup: {
      status: "failed",
      key: KEY,
      label: "Claude Code",
      failure: {
        message:
          "Claude Code did not finish starting within 120 seconds, so Oxyn stopped it. Nothing was sent.",
        category: "agentTimedOut",
        signIn: null,
        foundElsewhere: null,
        exit: null,
      },
    },
  },
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText("Claude Code could not start")).toBeVisible()
    await expect(canvas.getByText(/within 120 seconds/)).toBeVisible()
    // Never started again on its own (I-13): a click.
    await expect(args.onStart).not.toHaveBeenCalled()
    await userEvent.click(canvas.getByRole("button", { name: "Start again" }))
    await expect(args.onStart).toHaveBeenCalledOnce()
  },
}

/** The process died while starting: its last words, folded. */
export const Died: Story = {
  args: {
    startup: {
      status: "failed",
      key: KEY,
      label: "Claude Code",
      failure: {
        message: "the agent process stopped; asking again starts it again",
        category: "agentExited",
        signIn: null,
        foundElsewhere: null,
        exit: {
          code: 1,
          output:
            "npm ERR! code ENOTFOUND\nnpm ERR! network request to https://registry.npmjs.org failed",
        },
      },
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText("The agent stopped.")).toBeVisible()
    await userEvent.click(canvas.getByRole("button", { name: "Details" }))
    await expect(
      canvas.getByLabelText("The agent's last output")
    ).toHaveTextContent(/ENOTFOUND/)
  },
}

export const NotFound: Story = {
  args: {
    startup: {
      status: "failed",
      key: KEY,
      label: "Claude Code",
      failure: {
        message:
          "`npx` was not found; check the agent's command in the AI settings",
        category: "agentNotFound",
        signIn: null,
        foundElsewhere: "/opt/homebrew/bin/npx",
        exit: null,
      },
    },
  },
  play: async ({ canvasElement }) => {
    await expect(
      within(canvasElement).getByText("/opt/homebrew/bin/npx")
    ).toBeVisible()
  },
}

/** Refused before anything started: nothing to start again. */
export const Refused: Story = {
  args: {
    startup: {
      status: "failed",
      key: KEY,
      label: "Claude Code",
      failure: {
        message:
          "This connection is local-only, and Oxyn cannot see where an external agent sends its prompts. The agent was not started.",
        category: "refused",
        signIn: null,
        foundElsewhere: null,
        exit: null,
      },
    },
  },
  play: async ({ canvasElement }) => {
    await expect(
      within(canvasElement).queryByRole("button", { name: "Start again" })
    ).toBeNull()
  },
}

export const NeedsASignIn: Story = {
  args: {
    startup: {
      status: "failed",
      key: KEY,
      label: "Codex",
      failure: {
        message: "the agent needs you to sign in before it can answer",
        category: "agentSignIn",
        signIn: {
          agent: "Codex",
          methods: [
            {
              id: "chat-gpt",
              name: "ChatGPT",
              description: null,
              kind: "agent",
              command: null,
            },
          ],
          terminalCommand: "codex login",
        },
        foundElsewhere: null,
        exit: null,
      },
    },
  },
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText("Sign in to the agent")).toBeVisible()
    await userEvent.click(canvas.getByRole("button", { name: /ChatGPT/ }))
    await expect(args.onSignIn).toHaveBeenCalledWith("chat-gpt")
  },
}
