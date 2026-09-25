import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

import { connectionText } from "@/features/connections/copy-connection"
import {
  homonyms,
  hostileNames,
  manyConnections,
  summaries,
} from "./connection-fixtures"
import { SavedConnections } from "./saved-connections"

const meta = {
  title: "Oxyn/SavedConnections",
  component: SavedConnections,
  decorators: [
    (Story) => (
      <div className="max-w-md p-6">
        <Story />
      </div>
    ),
  ],
  args: {
    connections: summaries,
    opening: null,
    onOpen: fn(),
    onCancelOpening: fn(),
    onRetry: fn(),
  },
  // A connection is a button, which only admits phrasing content.
  afterEach: async ({ canvasElement }) => {
    await expect(canvasElement.querySelector("button :is(div, p)")).toBeNull()
  },
} satisfies Meta<typeof SavedConnections>

export default meta
type Story = StoryObj<typeof meta>

export const List: Story = {
  play: async ({ canvas, args }) => {
    await userEvent.click(canvas.getByText("billing replica"))
    await expect(args.onOpen).toHaveBeenCalledWith(summaries[1])
    // The driver's name, never its id; no identifier on screen (I-03).
    await expect(canvas.getAllByText(/PostgreSQL/).length).toBe(2)
    await expect(canvas.queryByText(/018f0000/)).toBeNull()
  },
}

/**
 * Connections whose workspace stays connected in the window (ADR-0046). A
 * console holding a transaction is said in words, naming the connection —
 * its locks are held while the user works elsewhere.
 */
export const OpenWithPendingTransaction: Story = {
  args: {
    openIds: [summaries[0]!.id, summaries[1]!.id, summaries[2]!.id],
    pendingTransactions: {
      [summaries[0]!.id]: "open",
      [summaries[1]!.id]: "unknown",
    },
  },
  play: async ({ canvas, args }) => {
    await expect(canvas.getAllByText("Open").length).toBe(3)
    await expect(
      canvas.getByRole("button", {
        name: /Transaction open in a console of billing/,
      })
    ).toBeVisible()
    await expect(
      canvas.getByRole("button", {
        name: /Transaction state unknown in a console of billing replica/,
      })
    ).toBeVisible()
    // An idle one says nothing about transactions.
    await expect(
      canvas.getByRole("button", { name: /scratch/ })
    ).not.toHaveAccessibleName(/Transaction/)
    await userEvent.click(
      canvas.getByRole("button", { name: /Transaction open/ })
    )
    await expect(args.onOpen).toHaveBeenCalledWith(summaries[0])
  },
}

/** Only open connections carry the marker: a closed one holds nothing. */
export const PendingIgnoredWhenNotOpen: Story = {
  args: { pendingTransactions: { [summaries[0]!.id]: "open" } },
  play: async ({ canvas }) => {
    await expect(canvas.queryByText("Open")).toBeNull()
    await expect(canvas.queryByText(/Transaction open/)).toBeNull()
  },
}

export const KeyboardOnly: Story = {
  play: async ({ canvas, args }) => {
    const [first, second] = canvas.getAllByRole("button")
    first!.focus()
    await userEvent.keyboard("{ArrowDown}")
    await waitFor(() => expect(second).toHaveFocus())
    await userEvent.keyboard("{Enter}")
    await expect(args.onOpen).toHaveBeenCalledWith(summaries[1])
  },
}

export const Opening: Story = {
  args: { opening: summaries[0]!.id },
  play: async ({ canvas, args }) => {
    // The others are inert; the opening one can be cancelled, Esc included.
    await expect(
      canvas.getByText("billing replica").closest("button")
    ).toBeDisabled()
    await userEvent.keyboard("{Escape}")
    await expect(args.onCancelOpening).toHaveBeenCalledOnce()
  },
}

export const Cancelling: Story = {
  args: { opening: summaries[0]!.id, cancelling: true },
}

export const Homonyms: Story = {
  args: { connections: homonyms },
  play: async ({ canvas }) => {
    await expect(canvas.getByText("db.internal:5432 / billing")).toBeVisible()
    await expect(
      canvas.getByText("db-eu.internal:5432 / billing")
    ).toBeVisible()
  },
}

export const HostileNames: Story = {
  args: { connections: hostileNames },
  play: async ({ canvasElement }) => {
    await expect(canvasElement.querySelector("img")).toBeNull()
  },
}

export const Loading: Story = {
  args: { connections: undefined },
}

export const NoneSaved: Story = {
  args: { connections: [] },
}

export const Failed: Story = {
  args: {
    connections: undefined,
    error: {
      message: "listing the saved connections: database is locked",
      retryable: true,
    },
  },
  play: async ({ canvas, args }) => {
    await userEvent.click(canvas.getByRole("button", { name: "Try again" }))
    await expect(args.onRetry).toHaveBeenCalledOnce()
  },
}

export const BackendAbsent: Story = {
  args: {
    connections: undefined,
    error: {
      message:
        "The Oxyn backend is not available here (list_connections): open the desktop application.",
      retryable: false,
    },
  },
}

export const Light: Story = { globals: { theme: "light" } }

/** Past five, the list folds and gains a filter. */
export const ManyConnections: Story = {
  args: { connections: manyConnections },
  play: async ({ canvas }) => {
    await expect(canvas.getAllByRole("listitem")).toHaveLength(5)
    const filter = canvas.getByRole("searchbox", {
      name: "Filter saved connections",
    })

    // Location and driver match as well as the name.
    await userEvent.type(filter, "scratch.sqlite")
    await expect(canvas.getAllByRole("listitem")).toHaveLength(7)
    await expect(canvas.queryByRole("button", { name: /Show all/ })).toBeNull()

    await userEvent.clear(filter)
    await userEvent.type(filter, "nothing like this")
    await expect(canvas.getByText(/No connection matches/)).toBeVisible()
    // Esc empties the filter instead of leaving the screen.
    await userEvent.keyboard("{Escape}")
    await expect(filter).toHaveValue("")

    await userEvent.type(filter, "nothing like this")
    await userEvent.click(canvas.getByRole("button", { name: "Clear filter" }))
    await expect(filter).toHaveValue("")

    await userEvent.click(
      canvas.getByRole("button", { name: "Show all 23 connections" })
    )
    await expect(canvas.getAllByRole("listitem")).toHaveLength(23)
  },
}

async function openMenuOn(name: string) {
  const row = within(document.body).getByText(name)
  await userEvent.pointer({ keys: "[MouseRight]", target: row })
  const page = within(document.body)
  await page.findByRole("menu")
  return page
}

async function closeMenu(page: ReturnType<typeof within>) {
  await userEvent.keyboard("{Escape}")
  await waitFor(() => expect(page.queryByRole("menu")).toBeNull())
}

const menuActions = {
  newConsole: fn(),
  refreshCatalog: fn(),
  edit: fn(),
  changeEnvironment: fn(),
  copy: fn(),
  delete: fn(),
  disconnect: fn(),
  duplicate: fn(),
}

/**
 * The menu of a closed connection: `Connect` is the click, `Refresh catalog`
 * says what it needs, `Duplicate` is the host's.
 */
export const ContextMenuOfAClosedConnection: Story = {
  args: {
    menuActions: () => ({ ...menuActions, disconnect: undefined }),
  },
  play: async ({ args }) => {
    const page = await openMenuOn("billing replica")
    await expect(
      page.queryByRole("menuitem", { name: "Disconnect" })
    ).toBeNull()
    const refresh = page.getByRole("menuitem", { name: /Refresh catalog/ })
    await expect(refresh).toHaveAttribute("aria-disabled", "true")
    await expect(refresh).toHaveTextContent("Connect to read its catalog")
    await userEvent.click(page.getByRole("menuitem", { name: "Duplicate" }))
    await expect(menuActions.duplicate).toHaveBeenCalled()
    await waitFor(() => expect(page.queryByRole("menu")).toBeNull())

    const again = await openMenuOn("billing replica")
    await userEvent.click(again.getByRole("menuitem", { name: "Connect" }))
    await expect(args.onOpen).toHaveBeenCalledWith(summaries[1])
    await waitFor(() => expect(again.queryByRole("menu")).toBeNull())
  },
}

/**
 * The connection whose workspace stayed open: `Disconnect` rather than
 * `Connect`, its catalog refreshable, and `Delete…` greyed until it is left.
 */
export const ContextMenuOfTheOpenConnection: Story = {
  args: {
    openIds: [summaries[0]!.id],
    menuActions: () => ({ ...menuActions, delete: undefined }),
  },
  play: async () => {
    const page = await openMenuOn("billing")
    await expect(page.queryByRole("menuitem", { name: "Connect" })).toBeNull()
    const remove = page.getByRole("menuitem", { name: /Delete/ })
    await expect(remove).toHaveAttribute("aria-disabled", "true")
    await expect(remove).toHaveTextContent("Disconnect it to delete it")
    await expect(
      page.getByRole("menuitem", { name: "Refresh catalog" })
    ).not.toHaveAttribute("aria-disabled", "true")
    await userEvent.click(page.getByRole("menuitem", { name: "Disconnect" }))
    await expect(menuActions.disconnect).toHaveBeenCalled()
    await waitFor(() => expect(page.queryByRole("menu")).toBeNull())
  },
}

/** While a connection opens, what would open, close or delete one waits. */
export const ContextMenuWhileOpening: Story = {
  args: {
    opening: summaries[0]!.id,
    menuActions: () => ({ ...menuActions, disconnect: undefined }),
  },
  play: async () => {
    // The rows are disabled buttons, which a right click does not reach: the
    // opening row's Cancel is what remains to point at.
    const page = within(document.body)
    await userEvent.pointer({
      keys: "[MouseRight]",
      target: page.getByRole("button", { name: /Cancel opening/ }),
    })
    await page.findByRole("menu")
    const connect = page.getByRole("menuitem", { name: /^Connect/ })
    await expect(connect).toHaveAttribute("aria-disabled", "true")
    await expect(connect).toHaveTextContent("The connection is busy")
    await expect(
      page.getByRole("menuitem", { name: "Edit…" })
    ).not.toHaveAttribute("aria-disabled", "true")
    await closeMenu(page)
  },
}

const copied = fn()

/**
 * `Copy connection` carries what the list shows — never a secret, its
 * reference or the connection's id (I-03). The text is the application's
 * own, composed by `connectionText`.
 */
export const CopyConnectionCarriesNoSecret: Story = {
  args: {
    menuActions: (connection) => ({
      copy: () => copied(connectionText(connection)),
    }),
  },
  play: async () => {
    const page = await openMenuOn("billing")
    await userEvent.click(
      page.getByRole("menuitem", { name: "Copy connection" })
    )
    await expect(copied).toHaveBeenCalledOnce()
    const text = String(copied.mock.calls[0]![0])
    await expect(text).toContain("Name: billing")
    await expect(text).toContain("Location: db.internal:5432 / billing")
    await expect(text).not.toContain(summaries[0]!.id)
    await expect(text).not.toMatch(/password|secret/i)
    await waitFor(() => expect(page.queryByRole("menu")).toBeNull())
  },
}

/** The connection opening stays in view even when folded away. */
export const OpeningAFoldedConnection: Story = {
  args: { connections: manyConnections, opening: manyConnections[20]!.id },
  play: async ({ canvas }) => {
    await expect(canvas.getAllByRole("listitem")).toHaveLength(6)
    await expect(
      canvas.getByRole("button", { name: /Cancel opening/ })
    ).toBeVisible()
  },
}
