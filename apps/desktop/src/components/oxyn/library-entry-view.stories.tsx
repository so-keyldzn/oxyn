import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, screen, userEvent, waitFor } from "storybook/test"

import { LibraryEntryView } from "./library-entry-view"
import type { HistoryDetail } from "@/lib/ipc/library"

// CodeMirror reads ⌘ on macOS and Ctrl elsewhere: pressing the wrong one would
// pass « runs nothing » for the wrong reason (see sql-editor.stories.tsx).
const MOD = /Mac/.test(navigator.platform) ? "Meta" : "Control"
const mod = (keys: string) => `{${MOD}>}${keys}{/${MOD}}`

const STATEMENT = [
  "SELECT c.name, sum(i.amount) AS total",
  "FROM invoices AS i",
  "JOIN customers AS c ON c.id = i.customer_id",
  "WHERE i.paid_at IS NULL",
  "GROUP BY c.name",
  "ORDER BY total DESC;",
].join("\n")

const entry: HistoryDetail = {
  id: 12,
  statement: STATEMENT,
  connectionName: "billing replica",
  status: "succeeded",
  error: null,
  needsInspection: false,
  fromAgent: false,
}

const meta = {
  title: "Oxyn/LibraryEntryView",
  component: LibraryEntryView,
  args: {
    open: true,
    state: { status: "history", entry },
    driver: "postgres",
    destination: "billing staging",
    onOpenCopy: fn(),
    onRetry: fn(),
    onClose: fn(),
  },
} satisfies Meta<typeof LibraryEntryView>

export default meta
type Story = StoryObj<typeof meta>

/** Found, then visible once the dialog's opening transition is over. */
async function shown(element: Promise<HTMLElement>) {
  const found = await element
  await waitFor(() => expect(found).toBeVisible())
  return found
}

/**
 * The full text reads, selects and copies; typing and ⌘↵ neither change nor
 * run it, and nothing opens until the named copy is chosen.
 */
export const History: Story = {
  play: async ({ args }) => {
    const editor = await screen.findByRole("textbox", { name: "SQL editor" })
    await expect(editor).toHaveTextContent("ORDER BY total DESC;")
    await userEvent.click(editor)
    await userEvent.keyboard(mod("a"))
    await userEvent.keyboard("DROP TABLE audit")
    await userEvent.keyboard(mod("{Enter}"))
    await expect(editor).toHaveTextContent("ORDER BY total DESC;")
    await expect(editor).not.toHaveTextContent("DROP TABLE audit")
    await expect(args.onOpenCopy).not.toHaveBeenCalled()
    await userEvent.click(
      screen.getByRole("button", { name: "Open copy in billing staging" })
    )
    await expect(args.onOpenCopy).toHaveBeenCalledTimes(1)
  },
}

/** A write whose outcome is unknown stays readable, warned, and uncopyable. */
export const NeedsInspection: Story = {
  args: {
    state: {
      status: "history",
      entry: {
        ...entry,
        statement: "UPDATE invoices SET paid_at = now() WHERE id = $1",
        connectionName: "billing primary",
        status: "failed",
        needsInspection: true,
      },
    },
  },
  play: async () => {
    await shown(screen.findByText("This write needs inspection"))
    await expect(
      screen.getByRole("textbox", { name: "SQL editor" })
    ).toHaveTextContent("UPDATE invoices")
    await expect(screen.queryByRole("button", { name: /Open copy/ })).toBeNull()
  },
}

/** An agent's statement says so, here as in the console it is copied into. */
export const FromAgent: Story = {
  args: { state: { status: "history", entry: { ...entry, fromAgent: true } } },
  play: async () => {
    await shown(screen.findByText("Written by an agent"))
  },
}

/** A changed working copy is readable without touching the named copy. */
export const SavedWithWorkingCopy: Story = {
  args: {
    state: {
      status: "saved",
      document: {
        id: "doc-1",
        title: "unpaid invoices.sql",
        text: "SELECT 'working'",
        savedTitle: "unpaid invoices.sql",
        savedText: "SELECT 'saved'",
        revision: 4,
        savedRevision: 3,
        isSaved: true,
        isOpen: false,
        connection: null,
        fromAgent: false,
      },
      entry: {
        id: "doc-1",
        title: "unpaid invoices.sql",
        connection: null,
        connectionName: "billing replica",
        updatedAt: "2026-09-14T17:00:00+00:00",
        isSaved: true,
        isOpen: false,
        hasChanges: true,
        fromAgent: false,
      },
    },
  },
  play: async () => {
    const editor = await screen.findByRole("textbox", { name: "SQL editor" })
    await expect(editor).toHaveTextContent("SELECT 'saved'")
    await userEvent.click(screen.getByRole("button", { name: "Working copy" }))
    await waitFor(() => expect(editor).toHaveTextContent("SELECT 'working'"))
  },
}

export const Loading: Story = {
  args: { state: { status: "loading" } },
  play: async () => {
    await shown(screen.findByText("Reading the full text…"))
    await expect(screen.queryByRole("button", { name: /Open copy/ })).toBeNull()
  },
}

export const Failed: Story = {
  args: {
    state: {
      status: "error",
      error: { message: "history entry 12 not found", retryable: false },
    },
  },
  play: async ({ args }) => {
    await shown(screen.findByText("history entry 12 not found"))
    await expect(screen.queryByRole("button", { name: "Try again" })).toBeNull()
    await expect(args.onRetry).not.toHaveBeenCalled()
  },
}
