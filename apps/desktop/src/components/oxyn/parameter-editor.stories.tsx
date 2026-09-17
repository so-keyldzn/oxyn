import * as React from "react"
import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, within } from "storybook/test"

import { ParameterEditor } from "./parameter-editor"
import type { ParameterRow } from "./parameter-editor"

const populated: Array<ParameterRow> = [
  { id: "a", type: "int64", text: "42" },
  { id: "b", type: "text", text: "paid" },
]

const meta = {
  title: "Oxyn/ParameterEditor",
  component: ParameterEditor,
  args: { rows: [], onChange: fn() },
  render: function Render(args) {
    const [rows, setRows] = React.useState(args.rows)
    return (
      <div className="w-[640px]">
        <ParameterEditor
          {...args}
          rows={rows}
          onChange={(next) => {
            setRows(next)
            args.onChange(next)
          }}
        />
      </div>
    )
  },
} satisfies Meta<typeof ParameterEditor>

export default meta
type Story = StoryObj<typeof meta>

export const Empty: Story = {
  play: async ({ canvas }) => {
    await userEvent.click(canvas.getByRole("button", { name: "Add parameter" }))
    // A new row binds NULL, and its value cannot be typed until a type is set.
    await expect(canvas.getByLabelText("Parameter 1 type")).toHaveValue("null")
    await expect(canvas.getByLabelText("Parameter 1 value")).toHaveAttribute(
      "readonly"
    )
  },
}

export const Populated: Story = {
  args: { rows: populated },
  play: async ({ canvas }) => {
    await userEvent.click(
      canvas.getByRole("button", { name: "Remove parameter 1" })
    )
    // The neighbour keeps its own type and value when a row above goes.
    await expect(canvas.getByLabelText("Parameter 1 type")).toHaveValue("text")
    await expect(canvas.getByLabelText("Parameter 1 value")).toHaveValue("paid")
  },
}

/**
 * At 420 px the row wraps instead of pushing the remove button out of the
 * panel: a value cannot be dropped if its button cannot be reached.
 */
export const Narrow: Story = {
  args: { rows: populated },
  decorators: [
    (Story) => (
      <div className="w-[420px] border">
        <Story />
      </div>
    ),
  ],
  play: async ({ canvas }) => {
    const panel = canvas.getByRole("region", { name: "Bound parameters" })
    await expect(panel.scrollWidth).toBeLessThanOrEqual(panel.clientWidth + 1)
    await expect(
      canvas.getByRole("button", { name: "Remove parameter 1" })
    ).toBeVisible()
  },
}

/** The backend refuses a value by position and type: the value never shows. */
export const InvalidValue: Story = {
  args: {
    rows: [{ id: "a", type: "uuid", text: "customer-4111-1111" }],
    error: "parameter 1 is not a valid UUID value",
  },
  play: async ({ canvas }) => {
    const alert = canvas.getByRole("alert")
    await expect(alert).toHaveTextContent("parameter 1")
    await expect(within(alert).queryByText(/customer-4111-1111/)).toBeNull()
  },
}
