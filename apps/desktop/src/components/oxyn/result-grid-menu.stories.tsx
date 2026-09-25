import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

import { invoiceColumns, syntheticPages } from "./fixtures"
import { ResultGrid } from "./result-grid"
import type { GridMenu } from "./result-grid-menu"

const ROWS = 40

function menuOf(overrides: Partial<GridMenu> = {}): GridMenu {
  return {
    origin: "query",
    relation: false,
    ai: { kind: "withheld", label: "Metadata" },
    copyText: fn(),
    copyRows: fn(),
    filterable: false,
    sortable: false,
    hideColumn: fn(),
    ...overrides,
  }
}

const meta = {
  title: "Oxyn/ResultGridMenu",
  component: ResultGrid,
  decorators: [
    (Story) => (
      <div className="h-[420px]">
        <Story />
      </div>
    ),
  ],
  args: {
    resultKey: "story-menu",
    columns: invoiceColumns,
    rowCount: ROWS,
    fetchPage: syntheticPages(ROWS, 0),
    onInspect: fn(),
    menu: menuOf(),
  },
} satisfies Meta<typeof ResultGrid>

export default meta
type Story = StoryObj<typeof meta>

const page = within(document.body)

async function cell(
  canvas: ReturnType<typeof within>,
  name: string | RegExp,
  index = 0
) {
  const cells = await canvas.findAllByRole("gridcell", { name })
  const found = cells[index]
  if (!found) throw new Error(`No cell ${String(name)}`)
  return found
}

async function rightClick(target: Element) {
  await userEvent.pointer({ keys: "[MouseRight]", target })
  await page.findByRole("menu")
}

async function menuClosed() {
  await waitFor(() => expect(page.queryByRole("menu")).toBeNull())
}

async function closeMenu() {
  await userEvent.keyboard("{Escape}")
  await menuClosed()
}

/**
 * A cell of a console's result: the right click moves the selection to it,
 * the filters are greyed — the SQL the user wrote is never rewritten —, and
 * `Copy value` copies the value as the grid shows it.
 */
export const ConsoleCell: Story = {
  play: async ({ canvas, args }) => {
    const globex = await cell(canvas, "Globex")
    await rightClick(globex)
    await expect(globex).toHaveAttribute("aria-selected", "true")
    const filter = page.getByRole("menuitem", { name: /Filter by this value/ })
    await expect(filter).toHaveAttribute("aria-disabled", "true")
    await expect(filter).toHaveTextContent(
      "The SQL you wrote is never rewritten"
    )
    await expect(
      page.getByRole("menuitem", { name: /Send to assistant/ })
    ).toHaveTextContent("The connection's AI level is Metadata")
    await userEvent.click(page.getByRole("menuitem", { name: /^Copy value/ }))
    await waitFor(() =>
      expect(args.menu?.copyText).toHaveBeenCalledWith("Globex", "Value")
    )
    await waitFor(() => expect(page.queryByRole("menu")).toBeNull())
  },
}

/**
 * A right click inside a selection keeps it: `Copy rows as ▸ CSV` asks for
 * its rows and shown columns, and `INSERT` says why it cannot name a table.
 */
export const CopySelectionAs: Story = {
  args: { menu: menuOf() },
  play: async ({ canvas, args }) => {
    // Rows 1 to 3, customer and amount.
    await userEvent.click(await cell(canvas, "Acme SA"))
    await userEvent.keyboard(
      "{Shift>}{ArrowDown}{ArrowDown}{ArrowRight}{/Shift}"
    )
    await rightClick(await cell(canvas, "Globex"))
    await userEvent.click(page.getByRole("menuitem", { name: /Copy rows as/ }))
    const insert = await page.findByRole("menuitem", { name: /^INSERT/ })
    await expect(insert).toHaveAttribute("aria-disabled", "true")
    await expect(insert).toHaveTextContent(
      "These rows do not come from one known table"
    )
    await userEvent.click(page.getByRole("menuitem", { name: /^CSV/ }))
    await waitFor(() =>
      expect(args.menu?.copyRows).toHaveBeenCalledWith({
        offset: 0,
        count: 3,
        columns: [1, 2],
        format: "csv",
        header: true,
        what: "3 rows as CSV",
      })
    )
    await menuClosed()
  },
}

/**
 * A truncated value is refused rather than shortened, like `⌘C`: the grid
 * says why, and nothing reaches the clipboard.
 */
export const TruncatedValueRefused: Story = {
  args: { menu: menuOf() },
  play: async ({ canvas, args }) => {
    const note = await cell(canvas, /^Long note/)
    await rightClick(note)
    await userEvent.click(page.getByRole("menuitem", { name: /^Copy value/ }))
    await menuClosed()
    await expect(
      await canvas.findByText(/Not copied: 1 selected value is truncated/)
    ).toBeInTheDocument()
    await expect(args.menu?.copyText).not.toHaveBeenCalled()
  },
}

/**
 * A cell of a table preview: `Sort ▸` orders the preview by the column, and
 * filtering by the value says what is still missing.
 */
export const PreviewCell: Story = {
  args: {
    menu: menuOf({
      origin: "preview",
      relation: true,
      ai: { kind: "none" },
      // As `object-view` wires it: the source declares both, and only the
      // order has a handler — the WHERE field is focused by `Filter…`.
      filterable: true,
      sortable: true,
      sort: fn(),
      filter: fn(),
    }),
  },
  play: async ({ canvas, args }) => {
    await rightClick(await cell(canvas, "Umbrella"))
    await expect(
      page.queryByRole("menuitem", { name: /Send to assistant/ })
    ).toBeNull()
    const filter = page.getByRole("menuitem", { name: /Filter by this value/ })
    await expect(filter).toHaveAttribute("aria-disabled", "true")
    await expect(filter).toHaveTextContent(
      "Not available yet: the preview cannot bind a value"
    )
    await userEvent.click(page.getByRole("menuitem", { name: /^Sort/ }))
    await userEvent.click(
      await page.findByRole("menuitem", { name: /^Descending/ })
    )
    await waitFor(() =>
      expect(args.menu?.sort).toHaveBeenCalledWith("customer", true)
    )
    await menuClosed()
  },
}

const WHERE_FIELD = "story-preview-where"

/**
 * `Filter…` on a preview's header brings the keyboard to the WHERE field, as
 * `object-view` wires it: the focus stays there once the menu has closed,
 * instead of going back to the grid.
 */
export const PreviewHeaderFilter: Story = {
  args: {
    menu: menuOf({
      origin: "preview",
      relation: true,
      filterable: true,
      sortable: true,
      sort: fn(),
      filter: () => document.getElementById(WHERE_FIELD)?.focus(),
    }),
  },
  render: (args) => (
    <div className="flex h-full flex-col">
      <input
        id={WHERE_FIELD}
        aria-label="Preview filter predicate"
        className="border"
      />
      <ResultGrid {...args} />
    </div>
  ),
  play: async ({ canvas }) => {
    await canvas.findAllByRole("gridcell", { name: "Globex" })
    await rightClick(canvas.getByRole("columnheader", { name: /^customer/ }))
    await userEvent.click(page.getByRole("menuitem", { name: /^Filter…/ }))
    await menuClosed()
    const field = canvas.getByRole("textbox", {
      name: "Preview filter predicate",
    })
    await waitFor(() => expect(field).toHaveFocus())
    // Still there after the menu's closing animation.
    await new Promise((resolve) => setTimeout(resolve, 300))
    await expect(field).toHaveFocus()
  },
}

/**
 * A column header: `Copy values` says it copies the loaded rows, never the
 * column (I-06), and asks for exactly those; `Freeze` is greyed with what is
 * missing.
 */
export const ColumnHeader: Story = {
  args: { menu: menuOf() },
  play: async ({ canvas, args }) => {
    await canvas.findAllByRole("gridcell", { name: "Globex" })
    const amount = canvas.getByRole("columnheader", { name: /^amount/ })
    await rightClick(amount)
    const freeze = page.getByRole("menuitem", { name: /^Freeze/ })
    await expect(freeze).toHaveAttribute("aria-disabled", "true")
    await expect(freeze).toHaveTextContent(
      "Not available yet: the grid cannot freeze a column"
    )
    await closeMenu()
    await rightClick(amount)
    const copyValues = page.getByRole("menuitem", { name: /^Copy values/ })
    await expect(copyValues).toHaveTextContent(`${ROWS} loaded rows`)
    await userEvent.click(copyValues)
    await waitFor(() =>
      expect(args.menu?.copyRows).toHaveBeenCalledWith({
        offset: 0,
        count: ROWS,
        columns: [2],
        format: "tsv",
        header: false,
        what: "Values of amount",
      })
    )
    await menuClosed()
    await rightClick(canvas.getByRole("columnheader", { name: /^customer/ }))
    await userEvent.click(page.getByRole("menuitem", { name: /^Hide/ }))
    await expect(args.menu?.hideColumn).toHaveBeenCalledWith(1)
    await menuClosed()
  },
}
