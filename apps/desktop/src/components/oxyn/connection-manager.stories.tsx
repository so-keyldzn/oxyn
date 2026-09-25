import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

import { homonyms, hostileNames, summaries } from "./connection-fixtures"
import { ConnectionManager } from "./connection-manager"

const meta = {
  title: "Oxyn/ConnectionManager",
  component: ConnectionManager,
  decorators: [
    (Story) => (
      <div className="max-w-2xl p-6">
        <Story />
      </div>
    ),
  ],
  args: {
    connections: summaries,
    onEdit: fn(),
    onDelete: fn(),
    onRetry: fn(),
  },
} satisfies Meta<typeof ConnectionManager>

export default meta
type Story = StoryObj<typeof meta>

export const Populated: Story = {
  play: async ({ canvas, args }) => {
    await userEvent.click(
      canvas.getByRole("button", { name: "Edit billing replica" })
    )
    await expect(args.onEdit).toHaveBeenCalledWith(summaries[1])
    await userEvent.click(
      canvas.getByRole("button", { name: "Delete billing" })
    )
    await expect(args.onDelete).toHaveBeenCalledWith(summaries[0])
    // The tier is written out, and no identifier reaches the screen (I-03).
    await expect(canvas.getAllByText(/AI · /).length).toBeGreaterThan(0)
    await expect(canvas.queryByText(/018f0000/)).toBeNull()
  },
}

export const ConnectionInUse: Story = {
  args: { openConnectionIds: [summaries[0]!.id] },
  play: async ({ canvas }) => {
    const remove = canvas.getByRole("button", { name: "Delete billing" })
    await expect(remove).toBeDisabled()
    // The disabled button says why, and the reason is a whole line: it was
    // cut off at 700 px when it trailed the location.
    await expect(remove).toHaveAccessibleDescription(
      "Open in this window: disconnect it to delete it."
    )
    await expect(
      canvas.getByRole("button", { name: "Edit billing" })
    ).toBeEnabled()
  },
}

const changeEnvironment = fn()
const disconnect = fn()
const refreshCatalog = fn()

/**
 * The context menu of a row, as Settings wires it: `Edit…` is the button,
 * `Disconnect`, `Refresh catalog` and `Change environment…` what the host
 * gives. What Settings cannot do says where it is done — a console and a
 * duplicate from the start screen —; the connection in use greys `Delete…`
 * until it is disconnected, from the same menu.
 */
export const ContextMenu: Story = {
  args: {
    openConnectionIds: [summaries[0]!.id],
    menuActions: (connection) => ({
      disconnect: connection.id === summaries[0]!.id ? disconnect : undefined,
      refreshCatalog:
        connection.id === summaries[0]!.id ? refreshCatalog : undefined,
      changeEnvironment,
      copy: fn(),
    }),
  },
  play: async ({ canvas, args }) => {
    const page = within(document.body)
    await userEvent.pointer({
      keys: "[MouseRight]",
      target: canvas.getByText("billing"),
    })
    await page.findByRole("menu")
    await expect(page.queryByRole("menuitem", { name: /^Connect/ })).toBeNull()
    await expect(
      page.getByRole("menuitem", { name: /^Refresh catalog/ })
    ).not.toHaveAttribute("aria-disabled", "true")
    const newConsole = page.getByRole("menuitem", { name: /^New console/ })
    await expect(newConsole).toHaveAttribute("aria-disabled", "true")
    await expect(newConsole).toHaveTextContent(
      "Show its workspace from the start screen, then press ⌘T"
    )
    await expect(
      page.getByRole("menuitem", { name: /^Duplicate/ })
    ).toHaveTextContent("Duplicate it from the start screen's connection list")
    const remove = page.getByRole("menuitem", { name: /^Delete/ })
    await expect(remove).toHaveAttribute("aria-disabled", "true")
    await expect(remove).toHaveTextContent(
      "Disconnect it first, from this menu"
    )
    await userEvent.click(page.getByRole("menuitem", { name: "Disconnect" }))
    await expect(disconnect).toHaveBeenCalled()
    await waitFor(() => expect(page.queryByRole("menu")).toBeNull())

    await userEvent.pointer({
      keys: "[MouseRight]",
      target: canvas.getByText("billing"),
    })
    await userEvent.click(
      await page.findByRole("menuitem", { name: "Change environment…" })
    )
    await expect(changeEnvironment).toHaveBeenCalled()
    await waitFor(() => expect(page.queryByRole("menu")).toBeNull())

    await userEvent.pointer({
      keys: "[MouseRight]",
      target: canvas.getByText("scratch"),
    })
    await page.findByRole("menu")
    await expect(
      page.getByRole("menuitem", { name: /^Connect/ })
    ).toHaveTextContent("Connect from the start screen")
    await userEvent.click(page.getByRole("menuitem", { name: "Delete…" }))
    await expect(args.onDelete).toHaveBeenCalledWith(summaries[2])
    await waitFor(() => expect(page.queryByRole("menu")).toBeNull())
  },
}

export const Homonyms: Story = {
  args: { connections: homonyms },
  play: async ({ canvas }) => {
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

export const Loading: Story = { args: { connections: undefined } }

export const Empty: Story = { args: { connections: [] } }

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
    await expect(args.onRetry).toHaveBeenCalled()
  },
}

export const Light: Story = { globals: { theme: "light" } }

export const Narrow: Story = {
  decorators: [
    (Story) => (
      <div className="w-[420px]">
        <Story />
      </div>
    ),
  ],
  args: { connections: hostileNames },
  /**
   * Long and hostile names at 420 px: the badges and actions wrap onto a
   * second line instead of pushing Edit and Delete past the edge.
   */
  play: async ({ canvasElement, canvas }) => {
    const room = canvasElement
      .querySelector(".w-\\[420px\\]")
      ?.getBoundingClientRect()
    for (const button of canvas.getAllByRole("button", { name: /^Delete / })) {
      await expect(button.getBoundingClientRect().right).toBeLessThanOrEqual(
        (room?.right ?? 0) + 1
      )
    }
  },
}
