import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, within } from "storybook/test"

import { AssistantErd } from "./assistant-erd"
import { AssistantMarkdown } from "./assistant-markdown"
import { shopLinks, shopTables } from "./erd-fixtures"

const source = "public.orders\ncustomers\nghost_table\nusers"

const meta = {
  title: "Oxyn/Assistant/ERD block",
  component: AssistantErd,
  args: {
    source,
    state: {
      status: "ready",
      tables: shopTables,
      links: shopLinks,
      omitted: 0,
      notFound: [],
      ambiguous: [],
      ignoredNames: 0,
    },
    onOpenObject: fn(),
    onRetry: fn(),
  },
  decorators: [
    (Story) => (
      <div className="max-w-2xl p-4">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof AssistantErd>

export default meta
type Story = StoryObj<typeof meta>

export const Populated: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    // The source is what the model wrote: one click away, as text.
    await userEvent.click(canvas.getByRole("button", { name: "Show source" }))
    await expect(canvas.getByLabelText("Diagram source")).toHaveTextContent(
      "ghost_table"
    )
    await userEvent.click(canvas.getByRole("button", { name: "Hide source" }))
  },
}

export const Loading: Story = {
  args: { state: { status: "loading" } },
}

export const UnknownTable: Story = {
  args: {
    state: {
      status: "ready",
      tables: shopTables.slice(0, 2),
      links: shopLinks.slice(0, 1),
      omitted: 0,
      notFound: ["ghost_table"],
      ambiguous: [
        { written: "users", candidates: ["auth.users", "public.users"] },
      ],
      ignoredNames: 3,
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    // Not found is said, never drawn from its spelling.
    await expect(
      canvas.getByText(/Not found among the objects Oxyn has read/)
    ).toBeVisible()
    await expect(canvas.queryByText("Open ghost_table")).toBeNull()
    await expect(
      canvas.getByText(/matches auth\.users, public\.users/)
    ).toBeVisible()
  },
}

export const NothingFound: Story = {
  args: {
    state: {
      status: "ready",
      tables: [],
      links: [],
      omitted: 0,
      notFound: ["ghost_table", "phantom"],
      ambiguous: [],
      ignoredNames: 0,
    },
  },
}

export const Failed: Story = {
  args: {
    state: {
      status: "error",
      message: "ERROR: permission denied for table orders (SQLSTATE 42501)",
    },
  },
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText(/SQLSTATE 42501/)).toBeVisible()
    await userEvent.click(canvas.getByRole("button", { name: "Try again" }))
    await expect(args.onRetry).toHaveBeenCalled()
  },
}

export const Light: Story = {
  globals: { theme: "light" },
  args: {
    state: {
      status: "ready",
      tables: shopTables,
      links: shopLinks,
      omitted: 0,
      notFound: ["ghost_table"],
      ambiguous: [],
      ignoredNames: 0,
    },
  },
}

/** In an answer: drawn once closed, code while it streams. */
export const InAnswer: Story = {
  render: (args) => (
    <AssistantMarkdown
      text={
        "The order tables:\n\n```erd\npublic.orders\ncustomers\n```\n\nStill writing:\n\n```erd\norder_items"
      }
      renderErd={(request) => (
        <AssistantErd
          source={request.source}
          state={args.state}
          onOpenObject={args.onOpenObject}
        />
      )}
    />
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getAllByText("Table diagram")).toHaveLength(1)
    // The open block is text until its fence closes.
    await expect(
      canvas.getByText("order_items", { selector: "code" })
    ).toBeVisible()
  },
}
