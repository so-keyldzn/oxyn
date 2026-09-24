import * as React from "react"
import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fireEvent, fn, userEvent, within } from "storybook/test"

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
    refusal: {
      position: 1,
      expectedType: "uuid",
      message: "parameter 1 is not a valid UUID value",
    },
  },
  play: async ({ canvas }) => {
    const alert = canvas.getByRole("alert")
    await expect(alert).toHaveTextContent("parameter 1")
    await expect(within(alert).queryByText(/customer-4111-1111/)).toBeNull()
  },
}

/**
 * The panel opens on the offending row: it is marked `aria-invalid`, brought
 * into view and handed the focus, so the person sees exactly what to fix
 * (UX-SPEC § Valeurs liées d'une console).
 */
export const FaultyRowFocused: Story = {
  args: {
    rows: populated,
    refusal: {
      position: 1,
      expectedType: "int64",
      message: "parameter 1 is not a valid Int64 value",
    },
  },
  play: async ({ canvas }) => {
    const field = canvas.getByLabelText("Parameter 1 value")
    await expect(field).toHaveAttribute("aria-invalid", "true")
    await expect(field).toHaveFocus()
    // The neighbour is untouched: only the named position is at fault.
    await expect(
      canvas.getByLabelText("Parameter 2 value")
    ).not.toHaveAttribute("aria-invalid", "true")
  },
}

/**
 * The clipboard is one of the six channels a bound value must never leave
 * through (I-03): the native `copy` stays cancelled on the value field.
 */
export const CopyRefused: Story = {
  args: { rows: populated },
  play: async ({ canvas }) => {
    const field = canvas.getByLabelText("Parameter 1 value")
    const notCancelled = await fireEvent.copy(field)
    // `dispatchEvent` returns `false` once a handler calls `preventDefault`.
    await expect(notCancelled).toBe(false)
  },
}

/** The cut channel stays cancelled on the value field, same as copy (I-03). */
export const CutRefused: Story = {
  args: { rows: populated },
  play: async ({ canvas }) => {
    const field = canvas.getByLabelText("Parameter 1 value")
    const notCancelled = await fireEvent.cut(field)
    await expect(notCancelled).toBe(false)
  },
}

/**
 * Dragging a selection out of the field is a distinct extraction channel
 * from the clipboard: it stays cancelled too (I-03).
 */
export const DragStartRefused: Story = {
  args: { rows: populated },
  play: async ({ canvas }) => {
    const field = canvas.getByLabelText("Parameter 1 value")
    const notCancelled = await fireEvent.dragStart(field)
    await expect(notCancelled).toBe(false)
  },
}

/**
 * The native context menu (Copy, Share, Services) is refused rather than
 * only its clipboard consequence: a webview's right-click menu is not
 * guaranteed to route through the `copy` event on every engine (I-03).
 */
export const ContextMenuRefused: Story = {
  args: { rows: populated },
  play: async ({ canvas }) => {
    const field = canvas.getByLabelText("Parameter 1 value")
    const notCancelled = await fireEvent.contextMenu(field)
    await expect(notCancelled).toBe(false)
  },
}

/**
 * This play only proves that the `[data-select="none"]` declaration in
 * styles.css wins the cascade. It does NOT prove that mouse selection is
 * actually prevented: `userEvent` in storybook/test simulates selection by
 * setting `selectionStart`/`selectionEnd` directly, which bypasses CSS
 * entirely, and Chromium does not honour `user-select` for a form field's
 * own content either way. A "real selection" play (double-click or drag,
 * then reading `getSelection()`/`selectionStart`) would be wrong in both
 * directions, so it is deliberately not written here.
 */
export const SelectionOff: Story = {
  args: { rows: populated },
  play: async ({ canvas }) => {
    const first = canvas.getByLabelText("Parameter 1 value")
    const second = canvas.getByLabelText("Parameter 2 value")
    await expect(getComputedStyle(first).getPropertyValue("user-select")).toBe(
      "none"
    )
    await expect(getComputedStyle(second).getPropertyValue("user-select")).toBe(
      "none"
    )
  },
}
