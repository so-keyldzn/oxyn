import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, within } from "storybook/test"

import { DriverChoices } from "./driver-choices"
import { catalogueDrivers, postgresDriver, sqliteDriver } from "./fixtures"

const meta = {
  title: "Oxyn/DriverChoices",
  component: DriverChoices,
  decorators: [
    (Story) => (
      <div className="max-w-2xl p-6">
        <Story />
      </div>
    ),
  ],
  args: {
    drivers: [postgresDriver, sqliteDriver],
    onChoose: fn(),
    onRetry: fn(),
  },
} satisfies Meta<typeof DriverChoices>

export default meta
type Story = StoryObj<typeof meta>

export const Compact: Story = {
  play: async ({ canvas, args }) => {
    await userEvent.click(canvas.getByRole("button", { name: /SQLite/ }))
    await expect(args.onChoose).toHaveBeenCalledWith(sqliteDriver)
    // What the driver needs is read from its fields: a file, or a server.
    await expect(canvas.getByText(/local file/)).toBeVisible()
    await expect(canvas.getByText(/port 5432/)).toBeVisible()
  },
}

export const Prominent: Story = { args: { prominent: true } }

/** A driver without a known mark still gets an icon, never an empty box. */
export const UnknownDriver: Story = {
  args: {
    prominent: true,
    drivers: [
      postgresDriver,
      sqliteDriver,
      { ...postgresDriver, id: "warehouse", displayName: "Warehouse" },
    ],
  },
}

export const Disabled: Story = {
  args: { disabled: true },
  play: async ({ canvas }) => {
    for (const button of canvas.getAllByRole("button")) {
      await expect(button).toBeDisabled()
    }
  },
}

export const Loading: Story = { args: { drivers: undefined } }

export const NoDriver: Story = { args: { drivers: [] } }

export const Failed: Story = {
  args: {
    drivers: undefined,
    error: {
      message:
        "The Oxyn backend is not available here (list_drivers): open the desktop application.",
      retryable: false,
    },
  },
}

/**
 * The catalogue the vision aims at: five tiles, the types in use first, and
 * the rest one search away.
 */
export const ManyTypes: Story = {
  args: { drivers: catalogueDrivers, used: ["sqlite"] },
  play: async ({ canvas, args }) => {
    const tiles = canvas.getAllByRole("listitem")
    await expect(tiles).toHaveLength(6)
    // The type already in use leads, though the build declares it last.
    await expect(tiles[0]).toHaveTextContent("SQLite")

    await userEvent.click(
      canvas.getByRole("button", { name: /All database types/ })
    )
    const palette = within(await within(document.body).findByRole("dialog"))
    await userEvent.type(palette.getByRole("combobox"), "vector")
    await expect(palette.getByText("Qdrant")).toBeVisible()
    await expect(palette.queryByText("MySQL")).toBeNull()

    await userEvent.clear(palette.getByRole("combobox"))
    await userEvent.type(palette.getByRole("combobox"), "no such base")
    await expect(palette.getByText("No database type matches.")).toBeVisible()

    await userEvent.clear(palette.getByRole("combobox"))
    await userEvent.type(palette.getByRole("combobox"), "neo")
    await userEvent.keyboard("{Enter}")
    await expect(args.onChoose).toHaveBeenCalledWith(
      expect.objectContaining({ displayName: "Neo4j" })
    )
  },
}

/** First launch with that many: the search is the page, no dialog. */
export const ManyTypesFirstLaunch: Story = {
  args: { drivers: catalogueDrivers, prominent: true },
  play: async ({ canvas, args }) => {
    await expect(canvas.queryByRole("dialog")).toBeNull()
    await userEvent.type(canvas.getByRole("combobox"), "duck")
    await userEvent.click(canvas.getByText("DuckDB"))
    await expect(args.onChoose).toHaveBeenCalledWith(
      expect.objectContaining({ displayName: "DuckDB" })
    )
  },
}
