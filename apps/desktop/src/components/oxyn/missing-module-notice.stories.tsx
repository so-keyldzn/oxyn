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
    shadows: missingModuleTable.virtualTable?.shadows ?? [],
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
    shadows: [],
    message: "no such module: spellfix1",
  },
  play: async ({ canvas }) => {
    await expect(
      canvas.getByText(/No table storing its data is listed/)
    ).toBeVisible()
    await expect(canvas.queryByRole("button", { name: /_/ })).toBeNull()
  },
}
