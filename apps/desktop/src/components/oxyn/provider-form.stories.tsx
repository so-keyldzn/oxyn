import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

import { ProviderForm } from "./provider-form"
import {
  externalAgent,
  remoteProvider,
  unresolvedProvider,
} from "./assistant-fixtures"
import { draftModels, listed, otherModels } from "./model-picker-fixtures"
import type { ListDraftModels } from "./use-draft-models"
import type { ModelListing } from "@/lib/ipc/ai"

const meta = {
  title: "Oxyn/Settings/ProviderForm",
  component: ProviderForm,
  args: {
    target: null,
    working: false,
    failure: null,
    onSaveProvider: fn(() => Promise.resolve(true)),
    onSaveAgent: fn(() => Promise.resolve(true)),
    onDone: fn(),
    onListModels: fn<ListDraftModels>(() =>
      Promise.resolve(listed(draftModels))
    ),
  },
  decorators: [
    (Story) => (
      <div className="max-w-2xl p-4">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof ProviderForm>

export default meta
type Story = StoryObj<typeof meta>

export const NewDeclaration: Story = {
  play: async ({ canvasElement }) => {
    // Its provider cannot stream yet: declaring it would only fail later.
    await expect(
      within(canvasElement).queryByRole("option", { name: "Gemini" })
    ).toBeNull()
  },
}

export const Saving: Story = {
  args: { working: true },
  play: async ({ canvasElement }) => {
    await expect(
      // The spinner lends its own name to the button.
      within(canvasElement).getByRole("button", { name: /Declare$/ })
    ).toBeDisabled()
  },
}

export const SavedThenCleared: Story = {
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await userEvent.selectOptions(canvas.getByLabelText("Kind"), "agent")
    await userEvent.type(canvas.getByLabelText("Name"), "Local agent")
    await expect(canvas.getByLabelText("Program")).toHaveAttribute(
      "placeholder",
      "claude-agent-acp"
    )
    await userEvent.type(canvas.getByLabelText("Program"), "my-agent")
    await userEvent.click(canvas.getByRole("button", { name: "Declare" }))
    await waitFor(() =>
      expect(args.onSaveAgent).toHaveBeenCalledWith({
        id: null,
        label: "Local agent",
        command: "my-agent",
        args: [],
        env: [],
      })
    )
    // Cleared once the backend said it was saved.
    await waitFor(() => expect(canvas.getByLabelText("Name")).toHaveValue(""))
  },
}

export const EnvironmentSentOnceThenForgotten: Story = {
  args: { onSaveAgent: fn(() => Promise.resolve(false)) },
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await userEvent.selectOptions(canvas.getByLabelText("Kind"), "agent")
    await userEvent.type(canvas.getByLabelText("Name"), "Gemini")
    await userEvent.type(canvas.getByLabelText("Program"), "gemini")
    await userEvent.type(
      canvas.getByLabelText("Arguments"),
      "--experimental-acp"
    )
    await userEvent.type(canvas.getByLabelText("Environment"), "no name here")
    await userEvent.click(canvas.getByRole("button", { name: "Declare" }))
    // A line that sets nothing is refused, not skipped.
    await expect(
      await canvas.findByText("Line 1 is not NAME=value.")
    ).toBeVisible()
    await expect(args.onSaveAgent).not.toHaveBeenCalled()

    await userEvent.clear(canvas.getByLabelText("Environment"))
    await userEvent.type(
      canvas.getByLabelText("Environment"),
      "GEMINI_API_KEY=not-a-real-key\nCUSTOM=synthetic-custom\nREGION=west"
    )
    await expect(
      canvas.getByRole("switch", { name: "Secret: GEMINI_API_KEY" })
    ).toBeChecked()
    await expect(
      canvas.getByRole("switch", { name: "Secret: GEMINI_API_KEY" })
    ).toHaveAttribute("aria-disabled", "true")
    await userEvent.click(
      canvas.getByRole("switch", { name: "Secret: CUSTOM" })
    )
    await expect(
      canvas.getByRole("switch", { name: "Secret: REGION" })
    ).not.toBeChecked()
    await expect(
      canvas.getByText(/Other values are stored in clear/)
    ).toBeVisible()
    await userEvent.click(canvas.getByRole("button", { name: "Declare" }))
    await waitFor(() =>
      expect(args.onSaveAgent).toHaveBeenCalledWith(
        expect.objectContaining({
          args: ["--experimental-acp"],
          env: [
            { name: "GEMINI_API_KEY", value: "not-a-real-key", secret: true },
            { name: "CUSTOM", value: "synthetic-custom", secret: true },
            { name: "REGION", value: "west", secret: false },
          ],
        })
      )
    )
    // Even though the save failed, the value is gone from the screen (I-03);
    // the rest of the form stays to try again.
    await waitFor(() =>
      expect(canvas.getByLabelText("Environment")).toHaveValue("")
    )
    await expect(canvas.getByLabelText("Name")).toHaveValue("Gemini")
  },
}

export const EditingAProvider: Story = {
  args: { target: { kind: "provider", provider: remoteProvider } },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByLabelText("Name")).toHaveFocus())
    await expect(canvas.getByLabelText("Kind")).toBeDisabled()
    await expect(
      canvas.getByRole("switch", { name: "Remove the stored key" })
    ).toBeInTheDocument()
    await expect(canvas.getByText(/change the endpoint/i)).toBeInTheDocument()
  },
}

/** A kind no longer offered still names a provider declared with it. */
export const EditingAGeminiProvider: Story = {
  args: { target: { kind: "provider", provider: unresolvedProvider } },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByLabelText("Kind")).toHaveDisplayValue("Gemini")
  },
}

export const ReplacingAnAgent: Story = {
  args: { target: { kind: "agent", agent: externalAgent } },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByLabelText("Name")).toHaveValue(externalAgent.label)
    await expect(canvas.getByLabelText("Arguments")).toHaveValue("")
    await expect(canvas.getByText(/2 arguments/)).toBeVisible()
  },
}

export const AgentNotSaved: Story = {
  args: {
    target: { kind: "agent", agent: externalAgent },
    failure: {
      operation: "save-agent",
      message: "saving the agent: database is locked",
      retryable: true,
      keyMustBeRetyped: false,
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText("Agent not saved")).toBeVisible()
    await expect(canvas.getByText("You can try again.")).toBeVisible()
  },
}

const page = () => within(document.body)
const modelInput = () => page().getByRole("combobox", { name: "Default model" })

/** Kind, name, endpoint, key: what a new remote provider needs to list. */
async function fillRemoteProvider(canvasElement: HTMLElement) {
  const canvas = within(canvasElement)
  await userEvent.selectOptions(canvas.getByLabelText("Kind"), "openai")
  await userEvent.type(canvas.getByLabelText("Name"), "Gateway")
  await userEvent.type(
    canvas.getByLabelText("Endpoint"),
    "https://llm.example.test/v1"
  )
}

/** The full flow: the list arrives once the key is typed, a model is picked. */
export const DeclareWithAListedModel: Story = {
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await fillRemoteProvider(canvasElement)
    await expect(
      canvas.getByText(/Fill in the endpoint and the API key/)
    ).toBeVisible()
    await userEvent.type(canvas.getByLabelText("API key"), "sk-synthetic")
    // Once, after the pause: not once per keystroke of the key.
    await waitFor(() => expect(args.onListModels).toHaveBeenCalledOnce())
    await expect(args.onListModels).toHaveBeenCalledWith(
      {
        id: null,
        kind: "openai",
        baseUrl: "https://llm.example.test/v1",
        key: "sk-synthetic",
      },
      false
    )
    await expect(await canvas.findByText("3 models available.")).toBeVisible()
    await userEvent.click(modelInput())
    await userEvent.click(await page().findByRole("option", { name: /^Lyra/ }))
    await waitFor(() => expect(modelInput()).toHaveValue("Lyra"))
    await userEvent.click(canvas.getByRole("button", { name: "Declare" }))
    await waitFor(() =>
      expect(args.onSaveProvider).toHaveBeenCalledWith({
        id: null,
        kind: "openai",
        label: "Gateway",
        baseUrl: "https://llm.example.test/v1",
        model: "model-lyra-1",
        key: "sk-synthetic",
        clearKey: false,
      })
    )
  },
}

/** A declared provider lists with its stored key; nothing typed is needed. */
export const TestConnectionSucceeds: Story = {
  args: { target: { kind: "provider", provider: remoteProvider } },
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await waitFor(() =>
      expect(args.onListModels).toHaveBeenCalledWith(
        {
          id: remoteProvider.id,
          kind: remoteProvider.kind,
          baseUrl: remoteProvider.endpoint,
          key: null,
        },
        false
      )
    )
    await userEvent.click(
      canvas.getByRole("button", { name: "Test connection" })
    )
    await expect(args.onListModels).toHaveBeenLastCalledWith(
      expect.objectContaining({ id: remoteProvider.id }),
      true
    )
    await expect(
      await canvas.findByText("Connected — 3 models available")
    ).toBeVisible()
    // Testing saves nothing.
    await expect(args.onSaveProvider).not.toHaveBeenCalled()
  },
}

export const TestConnectionFails: Story = {
  args: {
    target: { kind: "provider", provider: remoteProvider },
    onListModels: fn<ListDraftModels>(() =>
      Promise.resolve({
        status: "failed",
        reason: "unauthorized",
        message: "401 Unauthorized",
      })
    ),
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(
      await canvas.findByText("The provider refused the API key.")
    ).toBeVisible()
    await userEvent.click(
      canvas.getByRole("button", { name: "Test connection" })
    )
    await expect(
      await canvas.findByText("Connection failed — 401 Unauthorized")
    ).toBeVisible()
  },
}

/** « Refresh models » asks again, past the cache, and the list replaces the error. */
export const ErrorThenRefreshRetries: Story = {
  args: {
    target: { kind: "provider", provider: remoteProvider },
    // The automatic listing fails; the forced one, a moment later, answers.
    onListModels: fn<ListDraftModels>((_probe, refresh) =>
      Promise.resolve(
        refresh
          ? listed(draftModels)
          : {
              status: "failed",
              reason: "unreachable",
              message: "connection refused",
            }
      )
    ),
  },
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await expect(
      await canvas.findByText("The endpoint could not be reached.")
    ).toBeVisible()
    await userEvent.click(
      canvas.getByRole("button", { name: "Refresh models" })
    )
    await expect(args.onListModels).toHaveBeenLastCalledWith(
      expect.objectContaining({ id: remoteProvider.id }),
      true
    )
    await expect(await canvas.findByText("3 models available.")).toBeVisible()
  },
}

/** The stored model is no longer listed: it stays, flagged, and is saved as is. */
export const CurrentModelUnavailable: Story = {
  args: { target: { kind: "provider", provider: remoteProvider } },
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await expect(
      await canvas.findByText(/is not listed by the provider/)
    ).toBeVisible()
    await expect(modelInput()).toHaveValue(remoteProvider.model)
    await userEvent.click(canvas.getByRole("button", { name: "Save changes" }))
    await waitFor(() =>
      expect(args.onSaveProvider).toHaveBeenCalledWith(
        expect.objectContaining({ model: remoteProvider.model })
      )
    )
  },
}

/** A failed list never blocks a declaration: the id is typed. */
export const ManualModelIdWhenListingFails: Story = {
  args: {
    onListModels: fn<ListDraftModels>(() =>
      Promise.resolve({
        status: "failed",
        reason: "unsupported",
        message: "no model list here",
      })
    ),
  },
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await fillRemoteProvider(canvasElement)
    await userEvent.type(canvas.getByLabelText("API key"), "sk-synthetic")
    await expect(
      await canvas.findByText(/does not list its models/)
    ).toBeVisible()
    await userEvent.type(modelInput(), "my-deployment")
    await userEvent.keyboard("{Enter}")
    await waitFor(() => expect(modelInput()).toHaveValue("my-deployment"))
    await userEvent.click(canvas.getByRole("button", { name: "Declare" }))
    await waitFor(() =>
      expect(args.onSaveProvider).toHaveBeenCalledWith(
        expect.objectContaining({ model: "my-deployment" })
      )
    )
  },
}

function deferred() {
  let resolve: (listing: ModelListing) => void = () => undefined
  const promise = new Promise<ModelListing>((settle) => {
    resolve = settle
  })
  return { promise, resolve }
}

// One answer per endpoint, released by the story in the order it chooses.
let answers = new Map<string, ReturnType<typeof deferred>>()
function answerFor(baseUrl: string) {
  const known = answers.get(baseUrl)
  if (known) return known
  const created = deferred()
  answers.set(baseUrl, created)
  return created
}

/** Two endpoints, answered out of order: only the latest one is shown. */
export const StaleListingIgnored: Story = {
  args: {
    onListModels: fn<ListDraftModels>(
      (probe) => answerFor(probe.baseUrl).promise
    ),
  },
  beforeEach: () => {
    answers = new Map()
  },
  play: async ({ canvasElement, args }) => {
    const first = answerFor("http://localhost:11434/v1")
    const second = answerFor("http://127.0.0.1:1234/v1")
    const canvas = within(canvasElement)
    await userEvent.selectOptions(
      canvas.getByLabelText("Kind"),
      "openai_compatible"
    )
    const endpoint = canvas.getByLabelText("Endpoint")
    // Local: no key needed to list.
    await userEvent.type(endpoint, "http://localhost:11434/v1")
    await waitFor(() => expect(args.onListModels).toHaveBeenCalledTimes(1))
    await userEvent.clear(endpoint)
    await userEvent.type(endpoint, "http://127.0.0.1:1234/v1")
    await waitFor(() => expect(args.onListModels).toHaveBeenCalledTimes(2))
    second.resolve(listed(otherModels))
    await expect(await canvas.findByText("1 model available.")).toBeVisible()
    first.resolve(listed(draftModels))
    // The first answer arrives last, and changes nothing.
    await new Promise((settle) => setTimeout(settle, 50))
    await expect(canvas.getByText("1 model available.")).toBeVisible()
    await userEvent.click(modelInput())
    await expect(
      (await page().findAllByRole("option")).map((option) => option.textContent)
    ).toEqual(["Draco 5model-draco-5"])
    await userEvent.keyboard("{Escape}")
  },
}
