import * as React from "react"
import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

import { SqlEditor } from "./sql-editor"
import type { EditorTarget } from "./sql-editor"
import { nameAt } from "@/features/consoles/object-under-cursor"
import { useActionSource } from "@/lib/actions/context"
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

/** The active console as the registry reads it: Run is possible. */
function InConsole({ children }: { children: React.ReactNode }) {
  const nothing = () => undefined
  useActionSource(
    "console",
    { canRun: true, running: false, cancelling: false, writing: false },
    {
      run: nothing,
      runAll: nothing,
      explain: nothing,
      cancel: nothing,
      save: nothing,
    }
  )
  return children
}

const inConsole = (Story: () => React.ReactElement) => (
  <InConsole>
    <Story />
  </InConsole>
)

const openInvoices = fn()

/** What the console gives: its run, and `invoices` as the loaded object. */
const consoleMenu = {
  onRun: fn(),
  objectAt: (text: string, offset: number) => {
    const name = nameAt(text, offset)
    return name?.length === 1 && name[0]?.text === "invoices"
      ? openInvoices
      : null
  },
}

/** Where `word` is drawn, for a right click on it. */
function coordsOf(root: HTMLElement, word: string) {
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT)
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    const at = node.textContent?.indexOf(word) ?? -1
    if (at < 0) continue
    const range = document.createRange()
    range.setStart(node, at + 1)
    range.setEnd(node, at + 2)
    const box = range.getBoundingClientRect()
    return { clientX: box.left + box.width / 2, clientY: box.top + 2 }
  }
  throw new Error(`${word} is not drawn`)
}

async function rightClick(target: HTMLElement, word?: string) {
  await userEvent.pointer({
    keys: "[MouseRight]",
    target,
    coords: word ? coordsOf(target, word) : undefined,
  })
  const page = within(document.body)
  await page.findByRole("menu")
  return page
}

/**
 * Nothing selected: `Run selection` says why it cannot, `Run statement`
 * runs the statement at the caret — the scope of ⌘↵ made explicit — and
 * Cut has nothing to cut. `Format` waits for a formatter, and says so.
 */
export const MenuWithoutSelection: Story = {
  args: { menu: consoleMenu },
  decorators: [inConsole],
  play: async ({ canvas }) => {
    const editor = canvas.getByRole("textbox")
    let page = await rightClick(editor)
    const selection = page.getByRole("menuitem", { name: /Run selection/ })
    await expect(selection).toHaveAttribute("aria-disabled", "true")
    await expect(selection).toHaveTextContent("Nothing is selected")
    await expect(
      page.getByRole("menuitem", { name: /^Cut/ })
    ).toHaveTextContent("Nothing is selected")
    await expect(
      page.getByRole("menuitem", { name: /Format/ })
    ).toHaveTextContent("Oxyn has no SQL formatter")
    // No destination for the assistant here: the entry does not exist.
    await expect(
      page.queryByRole("menuitem", { name: /Ask assistant/ })
    ).toBeNull()
    await userEvent.click(page.getByRole("menuitem", { name: "Run statement" }))
    await expect(consoleMenu.onRun).toHaveBeenCalledWith(
      expect.objectContaining({ kind: "statement" })
    )
    await waitFor(() => expect(page.queryByRole("menu")).toBeNull())

    // The name under the pointer, resolved as a mention would.
    page = await rightClick(editor, "invoices")
    await userEvent.click(
      page.getByRole("menuitem", { name: "Open object under cursor" })
    )
    await expect(openInvoices).toHaveBeenCalled()
    await waitFor(() => expect(page.queryByRole("menu")).toBeNull())
  },
}

/**
 * With a selection, the right click keeps it: `Run selection` runs exactly
 * what was selected, not the statement under the pointer.
 */
const runSelected = fn<(target: EditorTarget) => void>()

export const MenuWithSelection: Story = {
  args: { menu: { ...consoleMenu, onRun: runSelected } },
  decorators: [inConsole],
  play: async ({ canvas }) => {
    const editor = canvas.getByRole("textbox")
    await userEvent.click(editor)
    await userEvent.keyboard("{Shift>}{ArrowRight}{ArrowRight}{/Shift}")
    const page = await rightClick(editor, "customers")
    const run = page.getByRole("menuitem", { name: "Run selection" })
    await expect(run).not.toHaveAttribute("aria-disabled", "true")
    await userEvent.click(run)
    const target = runSelected.mock.calls[0]?.[0]
    await expect(target).toMatchObject({ kind: "selection" })
    if (target?.kind === "selection")
      await expect(target.end - target.start).toBe(2)
    await waitFor(() => expect(page.queryByRole("menu")).toBeNull())
  },
}

/**
 * A read-only view runs nothing: its menu has no run scope at all, as ⌘↵
 * does nothing there, and only Copy changes nothing.
 */
export const ReadOnlyMenu: Story = {
  args: { readOnly: true, menu: consoleMenu },
  decorators: [inConsole],
  play: async ({ canvas }) => {
    const page = await rightClick(canvas.getByRole("textbox"))
    await expect(
      page.queryByRole("menuitem", { name: /Run selection/ })
    ).toBeNull()
    await expect(
      page.queryByRole("menuitem", { name: /Run statement/ })
    ).toBeNull()
    await expect(
      page.getByRole("menuitem", { name: /^Paste/ })
    ).toHaveTextContent("This editor is read-only")
    await userEvent.keyboard("{Escape}")
    await waitFor(() => expect(page.queryByRole("menu")).toBeNull())
  },
}

export const SQLite: Story = {
  args: {
    driver: "sqlite",
    value: "SELECT name FROM sqlite_master WHERE type = 'table';",
  },
}
