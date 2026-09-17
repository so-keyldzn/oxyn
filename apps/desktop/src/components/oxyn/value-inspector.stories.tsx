import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent } from "storybook/test"

import { invoiceColumns } from "./fixtures"
import { ValueInspector } from "./value-inspector"

const meta = {
  title: "Oxyn/ValueInspector",
  component: ValueInspector,
  decorators: [
    (Story) => (
      <div className="h-[480px] w-[320px] border">
        <Story />
      </div>
    ),
  ],
  args: {
    target: {
      row: 3,
      columns: invoiceColumns,
      cells: [
        "4",
        "Umbrella",
        "237.57",
        "2026-01-01T00:03:00.000Z",
        null,
        "NULL",
        { unrenderable: "unsupported type: Dictionary" },
      ],
    },
    column: 0,
    onSelectColumn: fn(),
    onInspect: fn(),
  },
} satisfies Meta<typeof ValueInspector>

export default meta
type Story = StoryObj<typeof meta>

export const NoSelection: Story = { args: { target: null, column: null } }

/**
 * The box keeps the focus while the arrows move the selection, so the
 * selected field is named by `aria-activedescendant` — nothing else tells a
 * screen reader which one it is.
 */
export const SelectedFieldIsNamed: Story = {
  args: { column: 2 },
  play: async ({ canvas }) => {
    const fields = canvas.getByRole("listbox", { name: "Fields of row 4" })
    const active = fields.getAttribute("aria-activedescendant")
    await expect(active).toBeTruthy()
    const option = document.getElementById(active ?? "")
    await expect(option).toHaveAttribute("aria-selected", "true")
    await expect(option).toHaveTextContent("amount")
  },
}

/** The row page is read from the existing result: the fields wait for it. */
export const Loading: Story = {
  args: { target: { row: 3, columns: invoiceColumns, cells: null } },
}

/** An absent value and the text `NULL` are not the same thing. */
export const Populated: Story = {
  play: async ({ canvas, args }) => {
    const fields = canvas.getByRole("listbox", { name: "Fields of row 4" })
    const options = canvas.getAllByRole("option")
    await expect(options[4]?.querySelector("[data-null]")).not.toBeNull()
    await expect(options[5]).toHaveTextContent("NULL")
    await expect(options[5]?.querySelector("[data-null]")).toBeNull()
    fields.focus()
    await userEvent.keyboard("{ArrowDown}")
    await expect(args.onSelectColumn).toHaveBeenCalledWith(1)
    await userEvent.keyboard("{Enter}")
    await expect(args.onInspect).toHaveBeenCalledWith(0)
  },
}
