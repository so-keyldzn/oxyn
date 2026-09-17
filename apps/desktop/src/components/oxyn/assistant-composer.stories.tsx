import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

import { AssistantComposer } from "./assistant-composer"

/** The prop's own signature: the backend answers now or later. */
type Submit = (question: string) => boolean | Promise<boolean>

const meta = {
  title: "Oxyn/Assistant/Composer",
  component: AssistantComposer,
  args: {
    running: false,
    onSubmit: fn<Submit>(() => true),
    onStop: fn(),
  },
  decorators: [
    (Story) => (
      <div className="max-w-xl p-4">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof AssistantComposer>

export default meta
type Story = StoryObj<typeof meta>

export const Initial: Story = {
  play: async ({ canvasElement, args }) => {
    const field = within(canvasElement).getByRole("textbox", {
      name: "Question for the assistant",
    })
    await userEvent.type(field, "Which tables have no primary key?")
    // Shift+Enter is a new line, not a send.
    await userEvent.keyboard("{Shift>}{Enter}{/Shift}")
    await expect(args.onSubmit).not.toHaveBeenCalled()
    await userEvent.keyboard("{Enter}")
    await expect(args.onSubmit).toHaveBeenCalledWith(
      "Which tables have no primary key?"
    )
    await expect(field).toHaveValue("")
  },
}

export const Running: Story = {
  args: { running: true, initialValue: "And the indexes?" },
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    const field = canvas.getByRole("textbox", {
      name: "Question for the assistant",
    })
    await userEvent.click(field)
    // A message typed while an answer runs is queued, not refused — and it
    // approves nothing that is waiting for a review.
    await expect(canvas.getByText(/queues/)).toBeVisible()
    await userEvent.keyboard("{Enter}")
    await expect(args.onSubmit).toHaveBeenCalledWith("And the indexes?")
    await expect(field).toHaveValue("")
    await userEvent.keyboard("{Escape}")
    await expect(args.onStop).toHaveBeenCalledOnce()
    await expect(
      canvas.getByRole("button", { name: "Stop the assistant" })
    ).toBeEnabled()
  },
}

export const StopRequested: Story = {
  args: { running: true, stopRequested: true },
}

export const Unavailable: Story = {
  args: {
    disabledReason:
      "This connection's privacy tier only admits a provider that resolved to this machine.",
  },
}

export const RefusedByBackend: Story = {
  args: {
    onSubmit: fn<Submit>(() => false),
    initialValue: "Drop the audit table",
  },
  play: async ({ canvasElement, args }) => {
    const field = within(canvasElement).getByRole("textbox", {
      name: "Question for the assistant",
    })
    await userEvent.click(field)
    await userEvent.keyboard("{Enter}")
    await expect(args.onSubmit).toHaveBeenCalled()
    // A refusal before anything started keeps the draft.
    await expect(field).toHaveValue("Drop the audit table")
  },
}

export const DoubleSubmit: Story = {
  args: {
    onSubmit: fn<Submit>(
      () =>
        new Promise<boolean>((resolve) => {
          window.setTimeout(() => resolve(true), 300)
        })
    ),
  },
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    const field = canvas.getByRole("textbox", {
      name: "Question for the assistant",
    })
    await userEvent.type(field, "Delete the archived invoices")
    // An impatient second Enter, then a click on Send, while the first is in
    // flight: a question asked twice is answered twice, and billed twice.
    await userEvent.keyboard("{Enter}{Enter}")
    await userEvent.click(canvas.getByRole("button", { name: "Send question" }))
    await waitFor(() => expect(field).toHaveValue(""))
    await expect(args.onSubmit).toHaveBeenCalledTimes(1)
  },
}

export const NarrowWindow: Story = {
  args: { initialValue: "Which tables have no primary key?" },
  decorators: [
    (Story) => (
      <div className="w-[320px] p-2">
        <Story />
      </div>
    ),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    // The hint and the send button share one row: neither may be pushed out.
    await expect(
      canvas.getByRole("button", { name: "Send question" })
    ).toBeVisible()
    await expect(canvasElement.scrollWidth).toBeLessThanOrEqual(
      canvasElement.clientWidth
    )
  },
}
