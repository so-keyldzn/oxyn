import * as React from "react"
import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

import { ProviderSettingsView } from "./provider-settings-view"
import {
  externalAgent,
  localProvider,
  remoteProvider,
  unresolvedProvider,
} from "./assistant-fixtures"
import { draftModels, listed } from "./model-picker-fixtures"
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

const codexPreset: AgentPresetDraft = {
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
  title: "Oxyn/Settings/ProviderSettings",
  component: ProviderSettingsView,
  args: {
    status: "ready",
    providers: [remoteProvider, localProvider, unresolvedProvider],
    agents: [externalAgent],
    presets: [claudePreset, codexPreset],
    presetStates: {},
    declaringPreset: null,
    working: null,
    failure: null,
    models: {},
    onSaveProvider: fn(() => Promise.resolve(true)),
    onSaveAgent: fn(() => Promise.resolve(true)),
    onDetectPreset: fn(),
    onDeclarePreset: fn(),
    onRemoveProvider: fn(),
    onRemoveAgent: fn(),
    onListModels: fn(),
    onListDraftModels: fn(() => Promise.resolve(listed(draftModels))),
    onDismissFailure: fn(),
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

/** A row's button is named after the provider, then its status. */
function row(canvasElement: HTMLElement, title: string) {
  return within(canvasElement).getByRole("button", {
    name: new RegExp(`^${title}, `),
  })
}

async function openRow(canvasElement: HTMLElement, title: string) {
  await userEvent.click(row(canvasElement, title))
  await expect(
    within(canvasElement).getByRole("heading", { level: 2, name: title })
  ).toBeVisible()
}

export const Loading: Story = {
  args: { status: "loading", providers: [], agents: [] },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByRole("status")).toHaveAccessibleName(
      "Loading providers"
    )
    // Nothing to open before the declarations are read.
    await expect(canvas.queryByRole("list")).toBeNull()
  },
}

/** Every status at once: connected, declared, checking, failed, not connected. */
export const Ready: Story = {
  args: {
    presetStates: { codex: { status: "detecting" } },
    models: {
      [remoteProvider.id]: { status: "ready", models: draftModels },
      [localProvider.id]: {
        status: "error",
        message: "error sending request for url (http://localhost:11434/v1)",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(
      canvas.getByRole("heading", { name: "Connect your AI provider" })
    ).toBeVisible()
    const list = canvas.getByRole("list", { name: "Providers" })
    await expect(within(list).getAllByRole("listitem")).toHaveLength(7)
    // The status is in the name, never in the colour alone.
    await expect(row(canvasElement, "Claude Code")).toHaveAccessibleName(
      "Claude Code, Declared"
    )
    await expect(row(canvasElement, "Codex")).toHaveAccessibleName(
      "Codex, Checking status…"
    )
    await expect(row(canvasElement, "Anthropic API")).toHaveAccessibleName(
      "Anthropic API, Connected"
    )
    await expect(
      row(canvasElement, "OpenAI-compatible endpoint")
    ).toHaveAccessibleName("OpenAI-compatible endpoint, Connection failed")
    await expect(row(canvasElement, "OpenAI API")).toHaveAccessibleName(
      "OpenAI API, Not connected"
    )
    // The models named are the ones the provider listed.
    await expect(canvas.getByText("Orion 2 · Lyra · Vega mini")).toBeVisible()
    await expect(canvas.getByText("4 of 7 declared")).toBeVisible()
    // Opaque ids are data, never text.
    await expect(canvasElement.textContent).not.toContain(remoteProvider.id)
  },
}

/** Opening the screen: detection runs, a listing is under way. */
export const Checking: Story = {
  args: {
    agents: [],
    presetStates: {
      "claude-code": { status: "detecting" },
      codex: { status: "detecting" },
    },
    models: { [remoteProvider.id]: { status: "loading" } },
  },
  play: async ({ canvasElement }) => {
    await expect(row(canvasElement, "Claude Code")).toHaveAccessibleName(
      "Claude Code, Checking status…"
    )
    await expect(row(canvasElement, "Anthropic API")).toHaveAccessibleName(
      "Anthropic API, Checking status…"
    )
  },
}

export const NothingDeclared: Story = {
  args: { providers: [], agents: [] },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText(/Nothing declared yet/)).toBeVisible()
    await expect(canvas.getByText("0 of 6 declared")).toBeVisible()
    // A family not offered any more only shows with a declaration.
    await expect(
      canvas.queryByRole("button", { name: /^Google Gemini API/ })
    ).toBeNull()
    // Each row says what it is, never a model Oxyn would recommend.
    await expect(
      canvas.getByText("Hosted API, with your own Anthropic key")
    ).toBeVisible()
  },
}

export const LoadFailed: Story = {
  args: {
    status: "error",
    providers: [],
    agents: [],
    loadError: "reading the local state: database is locked",
  },
  play: async ({ canvasElement }) => {
    await expect(
      within(canvasElement).getByText(
        "reading the local state: database is locked"
      )
    ).toBeVisible()
  },
}

/** One provider opened: its declarations, their models, the form. */
export const DrilledIntoAProvider: Story = {
  args: {
    models: {
      [unresolvedProvider.id]: {
        status: "error",
        message:
          "error sending request for url (https://llm-gateway.internal/models)",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await openRow(canvasElement, "Google Gemini API")
    // The way back has the focus, and gives it back to the row.
    const back = canvas.getByRole("button", { name: "All providers" })
    await waitFor(() => expect(back).toHaveFocus())
    // Never rounded: unresolved says so.
    await expect(
      canvas.getByText("Unresolved · measurement time unknown")
    ).toBeVisible()
    await expect(canvas.getByText("Key configured")).toBeVisible()
    await expect(
      canvas.getByRole("button", { name: "List models" })
    ).toBeVisible()
    await expect(
      canvas.getByRole("form", { name: "Declare a provider" })
    ).toBeVisible()
    await userEvent.click(back)
    await waitFor(() =>
      expect(row(canvasElement, "Google Gemini API")).toHaveFocus()
    )
  },
}

/** Enter and Space open a row: it is a button, not a clickable div. */
export const KeyboardOpensARow: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    row(canvasElement, "OpenAI API").focus()
    await userEvent.keyboard("{Enter}")
    await expect(
      canvas.getByRole("heading", { level: 2, name: "OpenAI API" })
    ).toBeVisible()
    await userEvent.click(canvas.getByRole("button", { name: "All providers" }))
    await waitFor(() => expect(row(canvasElement, "OpenAI API")).toHaveFocus())
    await userEvent.keyboard(" ")
    await expect(
      canvas.getByRole("heading", { level: 2, name: "OpenAI API" })
    ).toBeVisible()
  },
}

export const Saving: Story = {
  args: { working: "saving" },
  play: async ({ canvasElement }) => {
    await openRow(canvasElement, "Anthropic API")
    await expect(
      within(canvasElement).getByRole("button", { name: "Edit Work account" })
    ).toBeDisabled()
  },
}

export const SaveFailed: Story = {
  args: {
    failure: {
      operation: "save-provider",
      message: "writing the provider key: the keychain refused the write",
      retryable: false,
      keyMustBeRetyped: true,
    },
  },
  play: async ({ canvasElement }) => {
    await openRow(canvasElement, "Anthropic API")
    // Said in the form that sent it, with what failed.
    const form = within(
      within(canvasElement).getByRole("form", { name: "Declare a provider" })
    )
    await expect(form.getByText("Provider not saved")).toBeVisible()
    await expect(
      form.getByText("The key was not kept: type it again.")
    ).toBeVisible()
  },
}

export const RemovalFailed: Story = {
  args: {
    failure: {
      operation: "remove-agent",
      message: "removing the agent: database is locked",
      retryable: true,
      keyMustBeRetyped: false,
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText("Agent not removed")).toBeVisible()
    await openRow(canvasElement, "Custom agent")
    await expect(canvas.getByText("Agent not removed")).toBeVisible()
    const form = within(canvas.getByRole("form", { name: "Declare an agent" }))
    await expect(form.queryByText("Agent not removed")).toBeNull()
  },
}

export const NothingIsClearedBeforeTheSaveSucceeds: Story = {
  args: {
    providers: [],
    agents: [],
    onSaveProvider: fn(() => Promise.resolve(false)),
  },
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await openRow(canvasElement, "Anthropic API")
    await userEvent.type(canvas.getByLabelText("Name"), "Work")
    await userEvent.type(
      canvas.getByLabelText("Endpoint"),
      "https://api.anthropic.com"
    )
    await userEvent.type(
      canvas.getByLabelText("Default model"),
      "claude-sonnet-5"
    )
    await userEvent.type(canvas.getByLabelText("API key"), "sk-ant-refused")
    await userEvent.click(canvas.getByRole("button", { name: "Declare" }))
    await waitFor(() => expect(args.onSaveProvider).toHaveBeenCalled())
    // The key leaves the screen at once (I-03); the rest waits for success.
    await waitFor(() =>
      expect(canvas.getByLabelText("API key")).toHaveValue("")
    )
    await expect(canvas.getByLabelText("Name")).toHaveValue("Work")
    await expect(canvas.getByLabelText("Default model")).toHaveValue(
      "claude-sonnet-5"
    )
  },
}

export const EditLandsOnTheForm: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await openRow(canvasElement, "Anthropic API")
    await userEvent.click(
      canvas.getByRole("button", { name: "Edit Work account" })
    )
    await expect(
      canvas.getByRole("form", { name: "Edit “Work account”" })
    ).toBeVisible()
    await waitFor(() => expect(canvas.getByLabelText("Name")).toHaveFocus())
    await expect(canvas.getByLabelText("Name")).toHaveValue("Work account")
  },
}

export const ReplacingAnAgent: Story = {
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await openRow(canvasElement, "Claude Code")
    // A preset is declared by its own button: no form until a replacement.
    await expect(canvas.queryByRole("form")).toBeNull()
    await userEvent.click(
      canvas.getByRole("button", { name: `Replace… ${externalAgent.label}` })
    )
    const form = within(
      canvas.getByRole("form", { name: `Replace “${externalAgent.label}”` })
    )
    await expect(form.getByLabelText("Program")).toHaveValue(
      externalAgent.command
    )
    // What the IPC does not return is said, not silently dropped.
    await expect(form.getByText(/type the arguments again/)).toBeVisible()
    // One argument per line, exactly as typed (ADR-0026).
    await userEvent.type(
      form.getByLabelText("Arguments"),
      "-y{Enter}@agentclientprotocol/claude-agent-acp@0.78.0"
    )
    await userEvent.click(form.getByRole("button", { name: "Replace" }))
    await waitFor(() =>
      expect(args.onSaveAgent).toHaveBeenCalledWith(
        // The same id: replaced in one write, never declared beside it.
        expect.objectContaining({
          id: externalAgent.id,
          command: externalAgent.command,
          args: ["-y", "@agentclientprotocol/claude-agent-acp@0.78.0"],
        })
      )
    )
  },
}

/** By hand: a program, its arguments, its environment (ADR-0026). */
export const DeclaringAnAgentByHand: Story = {
  args: { providers: [], agents: [] },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await openRow(canvasElement, "Custom agent")
    const form = within(canvas.getByRole("form", { name: "Declare an agent" }))
    await expect(form.getByLabelText("Kind")).toHaveValue("agent")
    await expect(form.getByLabelText("Program")).toBeVisible()
  },
}

export const CredentialsInEndpointAreRefusedWhileTyping: Story = {
  args: { providers: [], agents: [] },
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await openRow(canvasElement, "Anthropic API")
    await userEvent.type(
      canvas.getByLabelText("Endpoint"),
      "https://alice:hunter2@api.example.com"
    )
    // At once, not when the form is complete.
    await expect(await canvas.findByText(CREDENTIALS_IN_ENDPOINT)).toBeVisible()
    await userEvent.type(canvas.getByLabelText("Name"), "Work")
    // Found first: the open list hides the rest of the form from roles.
    const declare = canvas.getByRole("button", { name: "Declare" })
    await userEvent.type(canvas.getByLabelText("Default model"), "gpt")
    await userEvent.click(declare)
    await expect(args.onSaveProvider).not.toHaveBeenCalled()
  },
}

export const TheKeyIsSentOnceAndForgotten: Story = {
  args: { providers: [], agents: [] },
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await openRow(canvasElement, "Anthropic API")
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
    await openRow(canvasElement, "Anthropic API")
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

/** « Sign in with Claude » opens the agent, where declaring is confirmed. */
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
      canvas.getByRole("button", { name: "Sign in with Claude" })
    )
    // Nothing is declared by opening it.
    await expect(args.onDeclarePreset).not.toHaveBeenCalled()
    await userEvent.click(
      canvas.getByRole("button", { name: "Declare Claude Code" })
    )
    // The command is proposed; the native confirmation still asks (ADR-0026).
    await expect(args.onDeclarePreset).toHaveBeenCalledWith(
      expect.objectContaining({ id: "claude-code" })
    )
  },
}

/** The screen with its declarations held in memory, as the backend would. */
function SavingScreen(
  props: React.ComponentProps<typeof ProviderSettingsView>
) {
  const [providers, setProviders] = React.useState(props.providers)
  return (
    <ProviderSettingsView
      {...props}
      providers={providers}
      onSaveProvider={async (draft) => {
        await props.onSaveProvider(draft)
        setProviders((all) => [
          ...all,
          {
            id: "openai-5e1ec7ed",
            label: draft.label,
            kind: draft.kind,
            endpoint: draft.baseUrl,
            endpointRedacted: false,
            model: draft.model,
            keyConfigured: draft.key !== null,
            reach: "remote",
            measuredAtMs: Date.UTC(2026, 9, 4, 9, 30),
          },
        ])
        return true
      }}
    />
  )
}

/** Pick a listed model, declare, edit again: the picked model is still the one. */
export const PickedModelSurvivesTheEdit: Story = {
  args: { providers: [], agents: [] },
  render: (args) => <SavingScreen {...args} />,
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    const page = within(document.body)
    const model = () => page.getByRole("combobox", { name: "Default model" })
    await openRow(canvasElement, "OpenAI API")
    // The provider's page starts the form on its family.
    await expect(canvas.getByLabelText("Kind")).toHaveValue("openai")
    await userEvent.type(canvas.getByLabelText("Name"), "Gateway")
    await userEvent.type(
      canvas.getByLabelText("Endpoint"),
      "https://llm.example.test/v1"
    )
    await userEvent.type(canvas.getByLabelText("API key"), "sk-synthetic")
    await expect(await canvas.findByText("3 models available.")).toBeVisible()
    await userEvent.click(model())
    await userEvent.click(
      await page.findByRole("option", { name: /^Vega mini/ })
    )
    await userEvent.click(canvas.getByRole("button", { name: "Declare" }))
    await waitFor(() =>
      expect(args.onSaveProvider).toHaveBeenCalledWith(
        expect.objectContaining({ model: "model-vega-mini" })
      )
    )

    await userEvent.click(
      await canvas.findByRole("button", { name: "Edit Gateway" })
    )
    await expect(
      await canvas.findByRole("heading", { name: "Edit “Gateway”" })
    ).toBeVisible()
    // Listed again with the stored key, and shown by its name.
    await waitFor(() => expect(model()).toHaveValue("Vega mini"))
    await userEvent.click(model())
    await expect(
      await page.findByRole("option", { name: /^Vega mini/ })
    ).toHaveAttribute("aria-selected", "true")
    await userEvent.keyboard("{Escape}")
  },
}

/** An id typed and never picked is the one saved, not the one it replaces. */
export const TypedModelIsSaved: Story = {
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await openRow(canvasElement, "Anthropic API")
    await userEvent.click(
      canvas.getByRole("button", { name: "Edit Work account" })
    )
    const model = canvas.getByLabelText("Default model")
    await expect(model).toHaveValue("claude-sonnet-5")
    // Found first: the open list hides the rest of the form from roles.
    const save = canvas.getByRole("button", { name: "Save changes" })
    await userEvent.clear(model)
    await userEvent.type(model, "my-deployment")
    // Straight to the button, the list still open: nothing was picked.
    await userEvent.click(save)
    await waitFor(() =>
      expect(args.onSaveProvider).toHaveBeenCalledWith(
        expect.objectContaining({ model: "my-deployment" })
      )
    )
  },
}

let answerFirstEndpoint = () => {}

/**
 * Refresh on a remote endpoint without a key, then edit the endpoint before
 * the answer: that answer is the first endpoint's list, never the second's.
 */
export const AnswerForThePreviousEndpointIsDropped: Story = {
  args: {
    providers: [],
    agents: [],
    onListDraftModels: fn(
      () =>
        new Promise((resolve) => {
          answerFirstEndpoint = () => resolve(listed(draftModels))
        })
    ),
  },
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await openRow(canvasElement, "Anthropic API")
    const endpoint = canvas.getByLabelText("Endpoint")
    await userEvent.type(endpoint, "https://a.example.test/v1")
    await userEvent.click(
      canvas.getByRole("button", { name: "Refresh models" })
    )
    await expect(await canvas.findByText("Listing models…")).toBeVisible()
    await expect(args.onListDraftModels).toHaveBeenCalledOnce()
    await userEvent.clear(endpoint)
    await userEvent.type(endpoint, "https://b.example.test/v1")
    answerFirstEndpoint()
    await expect(
      await canvas.findByText(/Fill in the endpoint and the API key/)
    ).toBeVisible()
    // Long enough for the dropped answer to have landed, had it been kept.
    await new Promise((resolve) => window.setTimeout(resolve, 50))
    await expect(
      canvas.queryByText("3 models available.")
    ).not.toBeInTheDocument()
  },
}

/**
 * The backend caches a list by endpoint, not by key: a new key lists afresh,
 * or it would be shown what the previous key was allowed to see.
 */
export const NewKeyListsAfresh: Story = {
  args: { providers: [], agents: [] },
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await openRow(canvasElement, "Anthropic API")
    await userEvent.type(
      canvas.getByLabelText("Endpoint"),
      "https://llm.example.test/v1"
    )
    await userEvent.type(canvas.getByLabelText("API key"), "sk-first")
    await expect(await canvas.findByText("3 models available.")).toBeVisible()
    await expect(args.onListDraftModels).toHaveBeenLastCalledWith(
      expect.objectContaining({ baseUrl: "https://llm.example.test/v1" }),
      false
    )
    await userEvent.type(canvas.getByLabelText("API key"), "-second")
    await waitFor(() => expect(args.onListDraftModels).toHaveBeenCalledTimes(2))
    await expect(args.onListDraftModels).toHaveBeenLastCalledWith(
      expect.objectContaining({ baseUrl: "https://llm.example.test/v1" }),
      true
    )
  },
}
