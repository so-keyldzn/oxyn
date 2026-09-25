import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

import {
  AssistantAnswerActions,
  AssistantAnswerMenu,
} from "./assistant-answer-actions"

const meta = {
  title: "Oxyn/Assistant/AnswerActions",
  component: AssistantAnswerActions,
  args: {
    text: "There are 1,204 active clients.",
    canRegenerate: true,
    onRegenerate: fn(),
    onCopy: fn(() => true),
  },
  decorators: [
    (Story) => (
      <div className="max-w-xl p-4">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof AssistantAnswerActions>

export default meta
type Story = StoryObj<typeof meta>

export const CopyAndRegenerate: Story = {
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await userEvent.click(canvas.getByRole("button", { name: "Copy answer" }))
    await expect(args.onCopy).toHaveBeenCalledWith(
      "There are 1,204 active clients."
    )
    await expect(
      canvas.getByRole("button", { name: "Copy answer: copied" })
    ).toBeVisible()
    await userEvent.click(
      canvas.getByRole("button", { name: "Regenerate answer" })
    )
    await expect(args.onRegenerate).toHaveBeenCalledOnce()
  },
}

export const CopyRefusedByTheBrowser: Story = {
  args: { onCopy: fn(() => false) },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(canvas.getByRole("button", { name: "Copy answer" }))
    // A refused clipboard must not show a tick.
    await expect(
      canvas.getByRole("button", { name: "Copy answer" })
    ).toBeVisible()
  },
}

export const WhileSomethingRuns: Story = {
  args: { canRegenerate: false },
  play: async ({ canvasElement }) => {
    await expect(
      within(canvasElement).getByRole("button", { name: "Regenerate answer" })
    ).toBeDisabled()
  },
}

const MARKDOWN = "There are **1,204** active `clients`."

/**
 * The answer's context menu: `Copy answer` copies it as it reads,
 * `Copy as Markdown` its source; `Regenerate answer` is greyed while an
 * answer is written, as the button is.
 */
export const ContextMenuOfAnAnswer: Story = {
  args: { text: MARKDOWN, canRegenerate: false },
  render: (args) => (
    <AssistantAnswerMenu
      text={args.text}
      answering={!args.canRegenerate}
      onCopy={args.onCopy}
      onRegenerate={args.onRegenerate}
    >
      <p>There are 1,204 active clients.</p>
    </AssistantAnswerMenu>
  ),
  play: async ({ canvasElement, args }) => {
    const page = within(document.body)
    await userEvent.pointer({
      keys: "[MouseRight]",
      target: within(canvasElement).getByText(
        "There are 1,204 active clients."
      ),
    })
    const regenerate = await page.findByRole("menuitem", {
      name: /Regenerate answer/,
    })
    await expect(regenerate).toHaveAttribute("aria-disabled", "true")
    await expect(regenerate).toHaveTextContent("An answer is being written")
    await userEvent.click(
      page.getByRole("menuitem", { name: "Copy as Markdown" })
    )
    await expect(args.onCopy).toHaveBeenCalledWith(MARKDOWN)
    await waitFor(() => expect(page.queryByRole("menu")).toBeNull())

    await userEvent.pointer({
      keys: "[MouseRight]",
      target: within(canvasElement).getByText(
        "There are 1,204 active clients."
      ),
    })
    await userEvent.click(
      await page.findByRole("menuitem", { name: "Copy answer" })
    )
    await expect(args.onCopy).toHaveBeenLastCalledWith(
      "There are 1,204 active clients."
    )
    await waitFor(() => expect(page.queryByRole("menu")).toBeNull())
  },
}
