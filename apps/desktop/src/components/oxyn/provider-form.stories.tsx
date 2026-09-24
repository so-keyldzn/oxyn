import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

import { ProviderForm } from "./provider-form"
import { externalAgent, remoteProvider } from "./assistant-fixtures"

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

export const NewDeclaration: Story = {}

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
      "GEMINI_API_KEY=not-a-real-key"
    )
    await userEvent.click(canvas.getByRole("button", { name: "Declare" }))
    await waitFor(() =>
      expect(args.onSaveAgent).toHaveBeenCalledWith(
        expect.objectContaining({
          args: ["--experimental-acp"],
          env: [{ name: "GEMINI_API_KEY", value: "not-a-real-key" }],
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
