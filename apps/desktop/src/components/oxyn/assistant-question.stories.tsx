import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

import { AssistantQuestion } from "./assistant-question"

const meta = {
  title: "Oxyn/Assistant/Question",
  component: AssistantQuestion,
  args: {
    text: "How many active clients?",
    versions: { position: 1, count: 1, previous: null, next: null },
    busy: false,
    onEdit: fn(() => true),
    onSelectVersion: fn(),
    onCopy: fn(() => true),
  },
  decorators: [
    (Story) => (
      <div className="max-w-xl p-4">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof AssistantQuestion>

export default meta
type Story = StoryObj<typeof meta>

export const Asked: Story = {
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await expect(
      canvas.queryByRole("group", { name: "Versions of this exchange" })
    ).toBeNull()
    await userEvent.click(canvas.getByRole("button", { name: "Copy question" }))
    await expect(args.onCopy).toHaveBeenCalledWith("How many active clients?")
  },
}

export const Edited: Story = {
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await userEvent.click(canvas.getByRole("button", { name: "Edit question" }))
    const field = canvas.getByRole("textbox", { name: "Edit your question" })
    await waitFor(() => expect(field).toHaveFocus())
    await userEvent.clear(field)
    await userEvent.type(field, "How many active clients last month?{Enter}")
    // Sent as another version: what followed the first one stays with it.
    await expect(args.onEdit).toHaveBeenCalledWith(
      "How many active clients last month?"
    )
  },
}

export const EditCancelledWithEscape: Story = {
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await userEvent.click(canvas.getByRole("button", { name: "Edit question" }))
    const field = canvas.getByRole("textbox", { name: "Edit your question" })
    await userEvent.type(field, " and last month{Escape}")
    await expect(args.onEdit).not.toHaveBeenCalled()
    await waitFor(() =>
      expect(
        canvas.getByRole("button", { name: "Edit question" })
      ).toHaveFocus()
    )
  },
}

export const SeveralVersions: Story = {
  args: { versions: { position: 2, count: 3, previous: 4, next: 9 } },
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText("2 / 3")).toBeVisible()
    await userEvent.click(
      canvas.getByRole("button", { name: "Previous version" })
    )
    await expect(args.onSelectVersion).toHaveBeenCalledWith(4)
  },
}

export const WhileAnAnswerRuns: Story = {
  args: {
    busy: true,
    versions: { position: 2, count: 2, previous: 1, next: null },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    // Editing mid-answer would fork the conversation in the middle of it.
    await expect(
      canvas.getByRole("button", { name: "Edit question" })
    ).toBeDisabled()
    await expect(
      canvas.getByRole("button", { name: "Previous version" })
    ).toBeDisabled()
  },
}

export const RefusedEditKeepsTheEditorOpen: Story = {
  args: { onEdit: fn(() => false) },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(canvas.getByRole("button", { name: "Edit question" }))
    const field = canvas.getByRole("textbox", { name: "Edit your question" })
    await userEvent.type(field, " today{Enter}")
    await expect(field).toBeVisible()
  },
}

export const LongAndRightToLeft: Story = {
  args: {
    text: "كم عدد العملاء النشطين في قاعدة البيانات هذه، وما هي الجداول التي يجب النظر إليها؟",
  },
}
