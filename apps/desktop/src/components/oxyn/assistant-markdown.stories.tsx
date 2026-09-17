import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, within } from "storybook/test"

import { AssistantMarkdown } from "./assistant-markdown"
import { answer, hostileAnswer } from "./assistant-fixtures"

const meta = {
  title: "Oxyn/Assistant/Markdown",
  component: AssistantMarkdown,
  args: { text: answer, onOpenSql: fn() },
  decorators: [
    (Story) => (
      <div className="max-w-xl p-4">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof AssistantMarkdown>

export default meta
type Story = StoryObj<typeof meta>

export const Answer: Story = {
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await userEvent.click(
      canvas.getByRole("button", { name: "Open in console" })
    )
    // The text, and nothing else: opening runs nothing (I-07).
    await expect(args.onOpenSql).toHaveBeenCalledWith(
      "SELECT count(*)\nFROM public.clients\nWHERE deleted_at IS NULL;"
    )
  },
}

export const Streaming: Story = {
  args: {
    text: "Let me look at the plan:\n\n```sql\nEXPLAIN SELECT *\nFROM orders",
  },
  play: async ({ canvasElement }) => {
    // An unterminated block is readable, and not yet a proposal.
    const canvas = within(canvasElement)
    await expect(canvas.getByText(/EXPLAIN SELECT/)).toBeVisible()
    await expect(
      canvas.queryByRole("button", { name: "Open in console" })
    ).toBeNull()
  },
}

export const Empty: Story = {
  args: { text: "" },
}

export const OpeningUnavailable: Story = {
  args: {
    openSqlDisabledReason:
      "This answer cannot be opened in a console: Oxyn cannot record where it came from.",
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(
      canvas.getByRole("button", { name: "Open in console" })
    ).toBeDisabled()
  },
}

export const HostileAnswer: Story = {
  args: { text: hostileAnswer },
  play: async ({ canvasElement }) => {
    const root = canvasElement.querySelector('[data-slot="assistant-markdown"]')
    await expect(root).not.toBeNull()
    // Nothing that loads, navigates or runs is ever rendered.
    await expect(
      root?.querySelectorAll(
        "a, img, script, iframe, object, embed, form, input"
      )
    ).toHaveLength(0)
    const withHandlers = Array.from(root?.querySelectorAll("*") ?? []).filter(
      (element) =>
        Array.from(element.attributes).some(
          (attribute) =>
            attribute.name.startsWith("on") ||
            attribute.value.trim().toLowerCase().startsWith("javascript:")
        )
    )
    await expect(withHandlers).toHaveLength(0)
    await expect((window as { __pwned?: boolean }).__pwned).toBeUndefined()
    // The hostile text stays readable, as text.
    const canvas = within(canvasElement)
    await expect(canvas.getByText(/<script>window.__pwned/)).toBeVisible()
    await expect(canvas.getByText("[image: tracker]")).toBeVisible()
  },
}
