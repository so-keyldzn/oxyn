import * as React from "react"
import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor } from "storybook/test"

import { SqlEditor } from "./sql-editor"
import { modKey } from "@/lib/actions/platform"

// ⌘ on macOS, Ctrl elsewhere: the registry's dispatcher, installed by the
// preview as in the window, reads it from the platform. Pressing ⌘
// unconditionally would run nothing on the Linux runners — passing for the
// wrong reason the stories that expect nothing.
const mod = (keys: string) => `{${modKey}>}${keys}{/${modKey}}`

const meta = {
  title: "Oxyn/SqlEditor",
  component: SqlEditor,
  args: {
    value:
      "-- Invoices unpaid for more than 30 days\nSELECT c.name, i.amount, i.issued_at\nFROM invoices AS i\nJOIN customers AS c ON c.id = i.customer_id\nWHERE i.paid_at IS NULL\n  AND i.issued_at < now() - interval '30 days'\nORDER BY i.issued_at;",
    driver: "postgres",
    onChange: fn(),
    onCancel: fn(),
  },
  render: function Render(args) {
    const [value, setValue] = React.useState(args.value)
    return (
      <div className="h-[320px] border">
        <SqlEditor
          {...args}
          value={value}
          onChange={(next) => {
            setValue(next)
            args.onChange(next)
          }}
        />
      </div>
    )
  },
} satisfies Meta<typeof SqlEditor>

export default meta
type Story = StoryObj<typeof meta>

export const PostgreSQL: Story = {
  play: async ({ canvas }) => {
    const editor = canvas.getByRole("textbox")
    // macOS neither corrects SQL nor curls its quotes (ADR-0041 § 8).
    await expect(editor).toHaveAttribute("spellcheck", "false")
    await expect(editor).toHaveAttribute("autocorrect", "off")
    await expect(editor).toHaveAttribute("autocapitalize", "off")
  },
}

export const RunningEscapeCancels: Story = {
  args: { running: true },
  play: async ({ canvas, args }) => {
    await userEvent.click(canvas.getByRole("textbox"))
    await userEvent.keyboard("{Escape}")
    await waitFor(() => expect(args.onCancel).toHaveBeenCalled())
  },
}

/**
 * ⌘/ comments the line in the editor — the zone with the focus wins over
 * the global `Keyboard shortcuts` — and runs once: the registry stops the
 * key before CodeMirror's own binding.
 */
export const CommandSlashTogglesTheComment: Story = {
  play: async ({ canvas, args }) => {
    // The cursor starts on the first line, a comment: ⌘/ uncomments it.
    await userEvent.click(canvas.getByRole("textbox"))
    await userEvent.keyboard(mod("/"))
    await waitFor(() =>
      expect(args.onChange).toHaveBeenLastCalledWith(
        expect.stringMatching(/^Invoices unpaid/)
      )
    )
    await expect(args.onChange).toHaveBeenCalledTimes(1)
  },
}

/** ⌘F searches the text of the editor that has the focus. */
export const CommandFFindsInTheEditor: Story = {
  play: async ({ canvas, canvasElement }) => {
    await userEvent.click(canvas.getByRole("textbox"))
    await userEvent.keyboard(mod("f"))
    await waitFor(() =>
      expect(canvasElement.querySelector(".cm-search")).not.toBeNull()
    )
  },
}

/**
 * A read-only view of a stored query: selection and copy work, and nothing
 * changes its text (docs/UX-SPEC.md, « Consultation locale des requêtes »).
 * ⌘↵ there is refused by the registry, which reads `data-read-only`.
 */
export const ReadOnlyChangesNothing: Story = {
  args: { readOnly: true },
  play: async ({ canvas, args, canvasElement }) => {
    await userEvent.click(canvas.getByRole("textbox"))
    await userEvent.keyboard(mod("/"))
    await expect(args.onChange).not.toHaveBeenCalled()
    await expect(
      canvasElement.querySelector('[data-action-zone="editor"]')
    ).toHaveAttribute("data-read-only", "true")
  },
}

export const SQLite: Story = {
  args: {
    driver: "sqlite",
    value: "SELECT name FROM sqlite_master WHERE type = 'table';",
  },
}
