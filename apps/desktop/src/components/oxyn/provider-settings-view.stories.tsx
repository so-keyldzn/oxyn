import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

import { ProviderSettingsView } from "./provider-settings-view"
import {
  externalAgent,
  localProvider,
  remoteProvider,
  unresolvedProvider,
} from "./assistant-fixtures"
import { CREDENTIALS_IN_ENDPOINT } from "./provider-settings-model"
import type { AgentPresetDraft } from "@/lib/ipc/ai"

const claudePreset: AgentPresetDraft = {
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

const meta = {
  title: "Oxyn/Settings/ProviderSettings",
  component: ProviderSettingsView,
  args: {
    status: "ready",
    providers: [remoteProvider, localProvider, unresolvedProvider],
    agents: [externalAgent],
    presets: [claudePreset],
    presetStates: {},
    declaringPreset: null,
    working: null,
    failure: null,
    models: {},
    onSaveProvider: fn(),
    onSaveAgent: fn(),
    onDetectPreset: fn(),
    onDeclarePreset: fn(),
    onRemoveProvider: fn(),
    onRemoveAgent: fn(),
    onCheckModels: fn(),
  },
  decorators: [
    (Story) => (
      <div className="max-w-2xl p-4">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof ProviderSettingsView>

export default meta
type Story = StoryObj<typeof meta>

export const Loading: Story = {
  args: { status: "loading", providers: [], agents: [] },
}

export const NothingDeclared: Story = {
  args: { providers: [], agents: [] },
  play: async ({ canvasElement }) => {
    await expect(
      within(canvasElement).getByText("No provider declared")
    ).toBeVisible()
  },
}

export const Declared: Story = {
  args: {
    models: {
      [remoteProvider.id]: {
        status: "ready",
        models: [
          {
            id: "claude-sonnet-5",
            displayName: "Claude Sonnet 5",
            contextWindow: null,
            cost: null,
            reasoningEfforts: [],
          },
          {
            id: "claude-opus-5",
            displayName: "Claude Opus 5",
            contextWindow: null,
            cost: null,
            reasoningEfforts: [],
          },
        ],
      },
      [unresolvedProvider.id]: {
        status: "error",
        message:
          "error sending request for url (https://llm-gateway.internal/models)",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    // Never rounded: unresolved says so.
    await expect(
      canvas.getByText("Unresolved · measurement time unknown")
    ).toBeVisible()
    await expect(canvas.getAllByText("Key configured").length).toBeGreaterThan(
      0
    )
    // Opaque ids are data, never text.
    await expect(canvasElement.textContent).not.toContain(remoteProvider.id)
  },
}

export const LoadFailed: Story = {
  args: {
    status: "error",
    providers: [],
    agents: [],
    loadError: "reading the local state: database is locked",
  },
}

export const Saving: Story = { args: { working: "saving" } }

export const SaveFailed: Story = {
  args: {
    failure: {
      message: "writing the provider key: the keychain refused the write",
      retryable: false,
      keyMustBeRetyped: true,
    },
  },
}

export const CredentialsInEndpointAreRefusedWhileTyping: Story = {
  args: { providers: [], agents: [] },
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await userEvent.type(
      canvas.getByLabelText("Endpoint"),
      "https://alice:hunter2@api.example.com"
    )
    // At once, not when the form is complete.
    await expect(await canvas.findByText(CREDENTIALS_IN_ENDPOINT)).toBeVisible()
    await userEvent.type(canvas.getByLabelText("Name"), "Work")
    await userEvent.type(canvas.getByLabelText("Default model"), "gpt")
    await userEvent.click(canvas.getByRole("button", { name: "Declare" }))
    await expect(args.onSaveProvider).not.toHaveBeenCalled()
  },
}

export const TheKeyIsSentOnceAndForgotten: Story = {
  args: { providers: [], agents: [] },
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await userEvent.type(canvas.getByLabelText("Name"), "Work")
    await userEvent.type(
      canvas.getByLabelText("Endpoint"),
      "https://api.anthropic.com"
    )
    await userEvent.type(
      canvas.getByLabelText("Default model"),
      "claude-sonnet-5"
    )
    await userEvent.type(
      canvas.getByLabelText("API key"),
      "sk-ant-secret-value"
    )
    await userEvent.click(canvas.getByRole("button", { name: "Declare" }))
    await waitFor(() =>
      expect(args.onSaveProvider).toHaveBeenCalledWith(
        expect.objectContaining({ key: "sk-ant-secret-value", id: null })
      )
    )
    // The screen no longer holds it (I-03).
    await waitFor(() =>
      expect(canvas.getByLabelText("API key")).toHaveValue("")
    )
    await expect(canvasElement.innerHTML).not.toContain("sk-ant-secret-value")
  },
}

export const RemovalNamesTheProvider: Story = {
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await userEvent.click(
      canvas.getByRole("button", { name: "Remove Work account" })
    )
    const dialog = within(document.body)
    const title = await dialog.findByText(
      "Remove the “Work account” declaration?"
    )
    // The dialog fades in: visible once its opening transition ends.
    await waitFor(() => expect(title).toBeVisible())
    await userEvent.click(dialog.getByRole("button", { name: "Cancel" }))
    await expect(args.onRemoveProvider).not.toHaveBeenCalled()
  },
}

export const AnAgentAlreadyInstalledIsProposed: Story = {
  args: {
    providers: [],
    agents: [],
    presetStates: {
      "claude-code": {
        status: "ready",
        draft: {
          ...claudePreset,
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
    await userEvent.click(
      canvas.getByRole("button", { name: "Declare Claude Code" })
    )
    // The command is proposed; the native confirmation still asks (ADR-0026).
    await expect(args.onDeclarePreset).toHaveBeenCalledWith(
      expect.objectContaining({ id: "claude-code" })
    )
  },
}
