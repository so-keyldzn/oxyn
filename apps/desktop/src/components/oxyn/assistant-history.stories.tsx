import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

import { AssistantHistory } from "./assistant-history"
import type { ThreadSummary } from "@/lib/ipc/ai"

const items: Array<ThreadSummary> = [
  {
    id: "3",
    title: "Slowest queries of the week",
    createdAtMs: Date.UTC(2026, 8, 15, 9, 0),
    updatedAtMs: Date.UTC(2026, 8, 15, 12, 40),
    exchanges: 6,
    running: true,
  },
  {
    id: "2",
    title: "How many active clients?",
    createdAtMs: Date.UTC(2026, 8, 14, 16, 0),
    updatedAtMs: Date.UTC(2026, 8, 14, 16, 20),
    exchanges: 2,
    running: false,
  },
  {
    id: "1",
    title: "",
    createdAtMs: Date.UTC(2026, 8, 13, 8, 0),
    updatedAtMs: Date.UTC(2026, 8, 13, 8, 5),
    exchanges: 1,
    running: false,
  },
]

const meta = {
  title: "Oxyn/Assistant/History",
  component: AssistantHistory,
  args: {
    history: { status: "ready", items },
    currentId: "2",
    onNew: fn(),
    onOpen: fn(),
    onRename: fn(async () => {}),
    onDelete: fn(async () => {}),
    onReload: fn(),
  },
  decorators: [
    (Story) => (
      <div className="flex h-[520px] max-w-md flex-col border">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof AssistantHistory>

export default meta
type Story = StoryObj<typeof meta>

export const Listed: Story = {
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText("Answering")).toBeVisible()
    await expect(canvas.getByText("Untitled conversation")).toBeVisible()
    // Nothing is saved to disk, and the list says so (I-11).
    await expect(canvas.getByText(/not saved to disk/)).toBeVisible()
    await userEvent.click(
      canvas.getByRole("button", { name: /^Slowest queries/ })
    )
    await expect(args.onOpen).toHaveBeenCalledWith("3")
  },
}

export const Loading: Story = {
  args: { history: { status: "loading", items: [] } },
  play: async ({ canvasElement }) => {
    await expect(
      within(canvasElement).getByRole("status", {
        name: "Loading conversations",
      })
    ).toBeVisible()
  },
}

export const NothingYet: Story = {
  args: { history: { status: "ready", items: [] }, currentId: null },
  play: async ({ canvasElement }) => {
    await expect(
      within(canvasElement).getByText("No conversation yet")
    ).toBeVisible()
  },
}

export const ListingFailed: Story = {
  args: {
    history: {
      status: "error",
      items: [],
      message: "the backend is not there",
    },
  },
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText("the backend is not there")).toBeVisible()
    await userEvent.click(canvas.getByRole("button", { name: "Try again" }))
    await expect(args.onReload).toHaveBeenCalledOnce()
  },
}

export const Renamed: Story = {
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await userEvent.click(
      canvas.getByRole("button", { name: "Rename How many active clients?" })
    )
    const field = canvas.getByRole("textbox", {
      name: "New title for How many active clients?",
    })
    await userEvent.clear(field)
    await userEvent.type(field, "Client counts{Enter}")
    await expect(args.onRename).toHaveBeenCalledWith("2", "Client counts")
  },
}

export const DeletionNamesTheConversation: Story = {
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await userEvent.click(
      canvas.getByRole("button", { name: "Delete Slowest queries of the week" })
    )
    const dialog = within(document.body)
    const title = await dialog.findByText(
      "Delete “Slowest queries of the week”?"
    )
    await waitFor(() => expect(title).toBeVisible())
    await expect(
      dialog.getByText(/an answer still running stops/)
    ).toBeVisible()
    await userEvent.click(
      dialog.getByRole("button", { name: "Delete conversation" })
    )
    await expect(args.onDelete).toHaveBeenCalledWith("3")
  },
}

export const HostileAndLongTitles: Story = {
  args: {
    history: {
      status: "ready",
      items: [
        {
          ...items[1]!,
          id: "9",
          title: "<img src=x onerror=alert(1)> ".repeat(4),
        },
        {
          ...items[1]!,
          id: "10",
          title: "ما هي الجداول التي تحتوي على بيانات العملاء؟",
        },
      ],
    },
    currentId: "9",
  },
  play: async ({ canvasElement }) => {
    // A title is text, whatever it contains.
    await expect(canvasElement.querySelector("img")).toBeNull()
  },
}
