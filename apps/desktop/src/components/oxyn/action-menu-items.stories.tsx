import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

import { ActionMenuContent } from "./action-menu-items"
import { ContextMenu, ContextMenuTrigger } from "@/components/ui/context-menu"
import type { ActionSources } from "@/lib/actions/context"
import type { Surface } from "@/lib/actions/context-menus"

/** A target to right-click, and the menu of `surface` on it. */
function Harness({
  surface,
  sources,
  details,
  title,
}: {
  surface: Surface
  sources: ActionSources
  details?: Partial<Record<string, string>>
  title?: string
}) {
  return (
    <ContextMenu>
      <ContextMenuTrigger
        render={
          <div className="flex h-40 w-72 items-center justify-center rounded-md border border-dashed text-sm text-muted-foreground" />
        }
      >
        Right-click here
      </ContextMenuTrigger>
      <ActionMenuContent
        surface={surface}
        sources={sources}
        details={details}
        title={title}
      />
    </ContextMenu>
  )
}

const meta = {
  title: "Oxyn/ActionMenuContent",
  component: Harness,
} satisfies Meta<typeof Harness>

export default meta
type Story = StoryObj<typeof meta>

async function openMenu(canvas: ReturnType<typeof within>) {
  await userEvent.pointer({
    keys: "[MouseRight]",
    target: canvas.getByText("Right-click here"),
  })
  const page = within(document.body)
  await page.findByRole("menu")
  return page
}

async function closeMenu(page: ReturnType<typeof within>) {
  await userEvent.keyboard("{Escape}")
  await waitFor(() => expect(page.queryByRole("menu")).toBeNull())
}

const copyValue = fn()

/**
 * A cell of a console's result: the preview filters are greyed with their
 * reason — the SQL the user wrote is never rewritten — and `INSERT` says why
 * it cannot name a table.
 */
export const GridCellOfAConsole: Story = {
  args: {
    surface: "gridCell",
    sources: {
      grid: {
        state: {
          target: "cell",
          origin: "query",
          filterable: true,
          sortable: true,
          selectedRows: 3,
          oneColumn: false,
          relation: false,
          loadedRows: 40,
          shownColumns: 4,
          ai: { kind: "withheld", label: "Schema only" },
        },
        actions: {
          copyValue,
          copyRows: fn(),
          inspect: fn(),
          sort: fn(),
          hideColumn: fn(),
        },
      },
    },
  },
  play: async ({ canvas }) => {
    const page = await openMenu(canvas)
    const filter = page.getByRole("menuitem", { name: /Filter by this value/ })
    await expect(filter).toHaveAttribute("aria-disabled", "true")
    await expect(filter).toHaveTextContent(
      "The SQL you wrote is never rewritten"
    )
    const send = page.getByRole("menuitem", { name: /Send to assistant/ })
    await expect(send).toHaveTextContent(
      "The connection's AI level is Schema only"
    )
    await userEvent.click(page.getByRole("menuitem", { name: "Copy value" }))
    await expect(copyValue).toHaveBeenCalled()
    await waitFor(() => expect(page.queryByRole("menu")).toBeNull())
  },
}

/**
 * The header of a column: `Copy values` says it copies the loaded rows,
 * never the column (I-06); `Freeze` is greyed with what is missing.
 */
export const GridHeader: Story = {
  args: {
    surface: "gridHeader",
    sources: {
      grid: {
        state: {
          target: "header",
          origin: "preview",
          filterable: true,
          sortable: true,
          selectedRows: 0,
          oneColumn: true,
          relation: true,
          loadedRows: 200,
          shownColumns: 4,
          ai: { kind: "none" },
        },
        actions: {
          sort: fn(),
          filter: fn(),
          hideColumn: fn(),
          autosize: fn(),
          copyName: fn(),
          copyValues: fn(),
        },
      },
    },
    details: { "column.copyValues": "200 loaded rows" },
  },
  play: async ({ canvas }) => {
    const page = await openMenu(canvas)
    await expect(
      page.getByRole("menuitem", { name: /Copy values/ })
    ).toHaveTextContent("200 loaded rows")
    await expect(
      page.getByRole("menuitem", { name: /Freeze/ })
    ).toHaveAttribute("aria-disabled", "true")
    await closeMenu(page)
  },
}

const copyCode = fn()

/** A code block of the assistant: copied or put in a console, never run (I-07). */
export const AssistantCodeNeverRuns: Story = {
  args: {
    surface: "assistantCode",
    sources: {
      assistant: {
        state: { answering: false },
        actions: { copyCode, openInConsole: fn() },
      },
    },
  },
  play: async ({ canvas }) => {
    const page = await openMenu(canvas)
    const names = page
      .getAllByRole("menuitem")
      .map((item) => item.textContent.trim())
    await expect(names).toEqual(["Copy code", "Open in console"])
    await userEvent.click(page.getByRole("menuitem", { name: "Copy code" }))
    await expect(copyCode).toHaveBeenCalled()
    await waitFor(() => expect(page.queryByRole("menu")).toBeNull())
  },
}

/** A submenu opens by the keyboard, and each of its entries is an action. */
export const CatalogCopyAs: Story = {
  args: {
    surface: "catalog",
    title: "invoices",
    sources: {
      catalogNode: {
        state: {
          relation: true,
          holdsRecords: true,
          schemaContext: false,
          definition: false,
          expanded: true,
          pin: "absent",
        },
        actions: {
          openData: fn(),
          viewStructure: fn(),
          viewDdl: fn(),
          copyQualifiedName: fn(),
          copyAs: fn(),
          collapseAll: fn(),
        },
      },
    },
  },
  play: async ({ canvas }) => {
    const page = await openMenu(canvas)
    await userEvent.click(page.getByRole("menuitem", { name: /Copy as/ }))
    const ddl = await page.findByRole("menuitem", { name: /^DDL/ })
    await expect(ddl).toHaveAttribute("aria-disabled", "true")
    await expect(ddl).toHaveTextContent(
      "This session does not provide object definitions"
    )
    // The first Escape closes the submenu, the second the menu.
    await userEvent.keyboard("{Escape}")
    await closeMenu(page)
  },
}
