import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, within } from "storybook/test"

import { AgentPresets } from "./agent-presets"
import type { AgentPresetDraft } from "@/lib/ipc/ai"

const claude: AgentPresetDraft = {
  id: "claude-code",
  label: "Claude Code",
  package: "@agentclientprotocol/claude-agent-acp",
  version: "0.78.0",
  command: "npx",
  args: ["-y", "@agentclientprotocol/claude-agent-acp@0.78.0"],
  env: [],
  detected: false,
  launcher: null,
  agentProgram: null,
  signIn: "claude auth login",
}

const codex: AgentPresetDraft = {
  id: "codex",
  label: "Codex",
  package: "@agentclientprotocol/codex-acp",
  version: "1.12.0",
  command: "npx",
  args: ["-y", "@agentclientprotocol/codex-acp@1.12.0"],
  env: [],
  detected: false,
  launcher: null,
  agentProgram: null,
  signIn: "codex login",
}

const meta = {
  title: "Oxyn/Settings/AgentPresets",
  component: AgentPresets,
  args: {
    presets: [claude, codex],
    states: {},
    declaring: null,
    onDetect: fn(),
    onDeclare: fn(),
  },
  decorators: [
    (Story) => (
      <div className="max-w-2xl p-4">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof AgentPresets>

export default meta
type Story = StoryObj<typeof meta>

export const BeforeDetecting: Story = {
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await expect(
      canvas.getAllByText(/Nothing is saved until you confirm/)
    ).toHaveLength(2)
    await userEvent.click(canvas.getAllByRole("button", { name: /Detect/ })[0]!)
    await expect(args.onDetect).toHaveBeenCalledWith("claude-code")
  },
}

export const Detecting: Story = {
  args: { states: { "claude-code": { status: "detecting" } } },
  play: async ({ canvasElement }) => {
    await expect(
      within(canvasElement).getAllByRole("button", { name: /Detect/ })[0]
    ).toBeDisabled()
  },
}

export const ClaudeCodeFound: Story = {
  args: {
    states: {
      "claude-code": {
        status: "ready",
        draft: {
          ...claude,
          detected: true,
          command: "/opt/homebrew/bin/npx",
          launcher: "/opt/homebrew/bin/npx",
          agentProgram: "/Users/someone/.local/bin/claude",
          env: [{ name: "PATH", value: "/opt/homebrew/bin:/usr/bin:/bin" }],
        },
      },
    },
  },
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await expect(
      canvas.getByText("/Users/someone/.local/bin/claude")
    ).toBeVisible()
    await expect(canvas.getByText(/Environment: PATH/)).toBeVisible()
    await userEvent.click(
      canvas.getByRole("button", { name: "Declare Claude Code" })
    )
    // The draft is proposed, never saved on its own: the click confirms it.
    await expect(args.onDeclare).toHaveBeenCalledWith(
      expect.objectContaining({ command: "/opt/homebrew/bin/npx" })
    )
  },
}

export const CodexWithoutItsOwnCommand: Story = {
  args: {
    states: {
      codex: {
        status: "ready",
        draft: {
          ...codex,
          detected: true,
          command: "/usr/local/bin/npx",
          launcher: "/usr/local/bin/npx",
          agentProgram: null,
        },
      },
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(
      canvas.getByText(/The adapter ships its own copy/)
    ).toBeVisible()
    await expect(canvas.getAllByText("codex login").length).toBeGreaterThan(0)
  },
}

export const LauncherMissing: Story = {
  args: {
    states: {
      "claude-code": {
        status: "ready",
        draft: {
          ...claude,
          detected: true,
          launcher: null,
          agentProgram: null,
        },
      },
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText(/Install Node/)).toBeVisible()
    await expect(
      canvas.getByRole("button", { name: "Declare Claude Code" })
    ).toBeDisabled()
  },
}

export const DetectionFailed: Story = {
  args: {
    states: {
      codex: {
        status: "error",
        message: "looking for the agent: permission denied",
      },
    },
  },
  play: async ({ canvasElement }) => {
    await expect(within(canvasElement).getByRole("alert")).toHaveTextContent(
      "permission denied"
    )
  },
}

export const Declaring: Story = {
  args: { declaring: "codex" },
  play: async ({ canvasElement }) => {
    await expect(
      within(canvasElement).getByRole("button", { name: /Declare Codex/ })
    ).toBeDisabled()
  },
}
