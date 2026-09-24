import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor } from "storybook/test"

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
