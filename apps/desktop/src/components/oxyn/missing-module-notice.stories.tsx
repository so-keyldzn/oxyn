import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent } from "storybook/test"

import { MissingModuleNotice } from "./missing-module-notice"
import {
  missingModuleMessage,
  missingModuleTable,
} from "./virtual-table-fixtures"

const meta = {
  title: "Oxyn/MissingModuleNotice",
  component: MissingModuleNotice,
  decorators: [
    (Story) => (
      <div className="h-[360px] w-[640px] bg-background">
        <Story />
      </div>
    ),
  ],
  args: {
    module: "vec0",
    subject: {
      type: "virtualTable",
      shadows: missingModuleTable.virtualTable?.shadows ?? [],
    },
    message: missingModuleMessage,
    onOpenShadow: fn(),
  },
} satisfies Meta<typeof MissingModuleNotice>

export default meta
type Story = StoryObj<typeof meta>

/** The Data tab of `chunks_vec`, read without sqlite-vec. */
export const WithShadowTables: Story = {
  play: async ({ canvas, args }) => {
    await expect(
      canvas.getByText(/provided by the SQLite extension/)
    ).toHaveTextContent(
      "This table is provided by the SQLite extension vec0, which Oxyn does not load."
    )
    // sqlite-vec is opt-in per connection, off by default: the way is said.
    await expect(
      canvas.getByText(
        "You can enable sqlite-vec for this connection in its settings, if you trust this file."
      )
    ).toBeVisible()
    // The shadow tables are where the data is readable: one click away.
    await userEvent.click(
      canvas.getByRole("button", { name: "chunks_vec_info" })
    )
    await expect(args.onOpenShadow).toHaveBeenCalledWith("chunks_vec_info")

    // The engine's words stay available, never replaced.
    await expect(canvas.queryByLabelText("Server message")).toBeNull()
    await userEvent.click(
      canvas.getByRole("button", { name: "Driver message" })
    )
    await expect(canvas.getByLabelText("Server message")).toHaveTextContent(
      "no such module: vec0"
    )
  },
}

export const WithoutShadowTables: Story = {
  args: {
    module: "spellfix1",
    subject: { type: "virtualTable", shadows: [] },
    message: "no such module: spellfix1",
  },
  play: async ({ canvas }) => {
    await expect(
      canvas.getByText(/No table storing its data is listed/)
    ).toBeVisible()
    await expect(canvas.queryByRole("button", { name: /_/ })).toBeNull()
    // The sqlite-vec sentence belongs to vec0 alone.
    await expect(canvas.queryByText(/enable sqlite-vec/)).toBeNull()
  },
}

/**
 * `CREATE VIRTUAL TABLE … USING VEC0(…)`: SQLite folds ASCII case, so this is
 * vec0, and the way to sqlite-vec is said all the same.
 */
export const UpperCaseVec0: Story = {
  args: { module: "VEC0", message: "no such module: VEC0" },
  play: async ({ canvas }) => {
    await expect(canvas.getByText(/enable sqlite-vec/)).toBeVisible()
  },
}

/**
 * A plain view whose definition reads `chunks_vec`: SQLite answers the same
 * `no such module`, but the view is not the virtual table — neither provided
 * by the extension nor stored in its shadow tables.
 */
export const ViewReadingAVirtualTable: Story = {
  args: { subject: { type: "dependent", noun: "view" } },
  play: async ({ canvas }) => {
    await expect(
      canvas.getByText("This view reads a table that needs an SQLite extension")
    ).toBeVisible()
    await expect(
      canvas.getByText(/reads a virtual table whose module/)
    ).toHaveTextContent(
      "This view reads a virtual table whose module vec0 is not loaded: Oxyn does not load that SQLite extension, so the read fails."
    )
    await expect(
      canvas.queryByText(/provided by the SQLite extension/)
    ).toBeNull()
    await expect(canvas.queryByText(/stored in|No table storing/)).toBeNull()
    // The way to vec0 is the same, whatever reads it.
    await expect(canvas.getByText(/enable sqlite-vec/)).toBeVisible()
  },
}
