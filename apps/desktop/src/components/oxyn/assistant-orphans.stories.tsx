import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, userEvent, within } from "storybook/test"

import { AssistantOrphans } from "./assistant-orphans"

const meta = {
  title: "Oxyn/Assistant/Orphans",
  component: AssistantOrphans,
  decorators: [
    (Story) => (
      <div className="w-80 p-3">
        <Story />
      </div>
    ),
  ],
  args: {
    state: {
      status: "ready",
      items: [
        {
          id: "0190aaaa-0000-7000-8000-000000000001",
          title: "Why are invoices duplicated?",
          connectionName: "billing (prod)",
          updatedAtMs: Date.UTC(2026, 7, 30, 9, 15),
          exchanges: 6,
        },
        {
          id: "0190aaaa-0000-7000-8000-000000000002",
          title: "",
          connectionName: null,
          updatedAtMs: Date.UTC(2026, 6, 2, 17, 40),
          exchanges: 1,
        },
      ],
    },
  },
} satisfies Meta<typeof AssistantOrphans>

export default meta
type Story = StoryObj<typeof meta>

/** Folded, then read: rows are text, under the name the connection had. */
export const ReadOnly: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(
      canvas.getByRole("button", { name: /From deleted connections/ })
    )
    const list = canvas.getByRole("list", {
      name: "Conversations of deleted connections",
    })
    await expect(list).toHaveTextContent("Why are invoices duplicated?")
    await expect(list).toHaveTextContent("on billing (prod)")
    await expect(list).toHaveTextContent("on a deleted connection")
    // Nothing opens, renames or deletes one of these.
    await expect(within(list).queryByRole("button")).toBeNull()
  },
}

/** A workspace that never deleted a connection shows nothing at all. */
export const None: Story = {
  args: { state: { status: "ready", items: [] } },
  play: async ({ canvasElement }) => {
    await expect(
      canvasElement.querySelector('[data-slot="assistant-orphans"]')
    ).toBeNull()
  },
}

export const Unreadable: Story = {
  args: {
    state: { status: "error", message: "The workspace file is locked." },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(
      canvas.getByRole("button", { name: /From deleted connections/ })
    )
    await expect(canvas.getByRole("alert")).toHaveTextContent(
      "could not be listed"
    )
  },
}
