import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

import { CatalogTree, addressKey } from "./catalog-tree"
import { catalog } from "./fixtures"
import {
  HOSTILE,
  hostileAddress,
  hugeCatalog,
  invoicesAddress,
  unusualCatalog,
} from "./metadata-fixtures"

const meta = {
  title: "Oxyn/CatalogTree",
  component: CatalogTree,
  decorators: [
    (Story) => (
      <div className="flex h-[480px] w-[280px] flex-col bg-sidebar p-2">
        <Story />
      </div>
    ),
  ],
  args: {
    nodes: catalog,
    loading: new Set<string>(),
    selected: null,
    onExpand: fn(),
    onSelect: fn(),
  },
} satisfies Meta<typeof CatalogTree>

export default meta
type Story = StoryObj<typeof meta>

export const Browse: Story = {
  play: async ({ canvas, args }) => {
    await userEvent.click(canvas.getByText("public"))
    await expect(canvas.getByText("invoices")).toBeVisible()
    // A hostile name is shown as text, never interpreted.
    await expect(canvas.getByText('users"; DROP TABLE audit; --')).toBeVisible()

    // Expanding an unloaded schema asks for that level, and only that one.
    await userEvent.click(canvas.getByText("reporting"))
    await expect(args.onExpand).toHaveBeenCalledWith(catalog[1]?.address)

    await userEvent.click(canvas.getByText("invoices"))
    await expect(args.onSelect).toHaveBeenCalled()
  },
}

export const KeyboardOnly: Story = {
  play: async ({ canvas, args }) => {
    const tree = canvas.getByRole("tree")
    tree.focus()
    await userEvent.keyboard("{ArrowRight}{ArrowDown}{Enter}")
    await waitFor(() => expect(args.onSelect).toHaveBeenCalled())
  },
}

export const Filtered: Story = {
  play: async ({ canvas }) => {
    await userEvent.type(canvas.getByLabelText("Filter loaded objects"), "line")
    await waitFor(() => expect(canvas.getByText("invoice_lines")).toBeVisible())
    await expect(canvas.queryByText("customers")).toBeNull()
  },
}

export const LoadingSchema: Story = {
  args: { loading: new Set([addressKey(catalog[1]!.address)]) },
}

export const NothingLoaded: Story = {
  args: { nodes: [] },
}

/** System schemas are hidden from view, never removed from the catalog. */
export const HideSystemObjects: Story = {
  play: async ({ canvas }) => {
    await expect(canvas.getByText("pg_catalog")).toBeVisible()
    await userEvent.click(
      canvas.getByRole("button", { name: "Hide system objects" })
    )
    await expect(canvas.queryByText("pg_catalog")).toBeNull()
    await userEvent.click(
      canvas.getByRole("button", { name: "Show system objects" })
    )
    await expect(canvas.getByText("pg_catalog")).toBeVisible()
  },
}

/** A DDL invalidated this level: its old children stay, marked stale. */
export const StaleLevel: Story = {
  args: {
    nodes: catalog.map((node, index) =>
      index === 0 ? { ...node, stale: true } : node
    ),
  },
  play: async ({ canvas, args }) => {
    await expect(canvas.getByText("stale")).toBeVisible()
    await userEvent.click(canvas.getByText("public"))
    await expect(canvas.getByText("invoices")).toBeVisible()
    // Opening a stale level reads it again.
    await expect(args.onExpand).toHaveBeenCalledWith(catalog[0]?.address)
  },
}

const onQuery = fn()

export const SearchHits: Story = {
  args: {
    search: {
      hits: [
        {
          address: invoicesAddress,
          name: "invoices",
          kind: "table",
          holdsRecords: true,
          matched: "fieldName",
          matchedFields: ["customer_id"],
        },
        {
          address: hostileAddress,
          name: HOSTILE,
          kind: "table",
          holdsRecords: true,
          matched: "relationName",
          matchedFields: [],
        },
      ],
      searching: false,
      onQuery,
    },
  },
  play: async ({ canvas, args }) => {
    await userEvent.type(canvas.getByLabelText("Filter loaded objects"), "cust")
    await waitFor(() => expect(onQuery).toHaveBeenCalledWith("cust"))
    const hits = canvas.getByRole("list", { name: "Matching objects" })
    await expect(within(hits).getByText(HOSTILE)).toBeVisible()
    await userEvent.click(within(hits).getByText(HOSTILE))
    await expect(args.onSelect).toHaveBeenCalledWith(
      expect.objectContaining({ address: hostileAddress })
    )
  },
}

export const NoSearchHit: Story = {
  args: { search: { hits: [], searching: false, onQuery: fn() } },
  play: async ({ canvas }) => {
    await userEvent.type(canvas.getByLabelText("Filter loaded objects"), "zzz")
    await waitFor(() =>
      expect(
        canvas.getByText(/only covers what has already been read/)
      ).toBeVisible()
    )
  },
}

/**
 * The menu hands the node over: the qualified name is the backend's, never
 * assembled here from a hostile segment (I-10).
 */
export const ContextMenu: Story = {
  args: { onOpen: fn(), onCopyName: fn(), onRefresh: fn() },
  play: async ({ canvas, args }) => {
    await userEvent.click(canvas.getByText("public"))
    await userEvent.pointer({
      keys: "[MouseRight]",
      target: canvas.getByText(HOSTILE),
    })
    const menu = within(document.body)
    await userEvent.click(
      await menu.findByRole("menuitem", { name: "Copy qualified name" })
    )
    await expect(args.onCopyName).toHaveBeenCalledWith(
      expect.objectContaining({ name: HOSTILE })
    )
    await waitFor(() => expect(menu.queryByRole("menu")).toBeNull())
  },
}

/** Opens the context menu of `invoices` and returns where the menu renders. */
async function openInvoicesMenu(canvas: ReturnType<typeof within>) {
  await userEvent.click(canvas.getByText("public"))
  await userEvent.pointer({
    keys: "[MouseRight]",
    target: canvas.getByText("invoices"),
  })
  const menu = within(document.body)
  await menu.findByRole("menuitem", { name: "Copy qualified name" })
  return menu
}

/** Under `sampled`, to a built-in provider, an object can be pinned. */
export const PinToQuestion: Story = {
  args: {
    onCopyName: fn(),
    pin: { tier: "sampled", destination: "provider", onPin: fn() },
  },
  play: async ({ canvas, args }) => {
    const menu = await openInvoicesMenu(canvas)
    await userEvent.click(
      menu.getByRole("menuitem", { name: "Pin to question" })
    )
    await expect(args.pin?.onPin).toHaveBeenCalledWith(
      expect.objectContaining({ name: "invoices" })
    )
    await waitFor(() => expect(menu.queryByRole("menu")).toBeNull())
  },
}

/**
 * Under `metadata`, no row can leave: the action is absent, not greyed — a
 * disabled item would advertise a path the tier closes.
 */
export const NoPinUnderMetadata: Story = {
  args: {
    onCopyName: fn(),
    pin: { tier: "metadata", destination: "provider", onPin: fn() },
  },
  play: async ({ canvas }) => {
    const menu = await openInvoicesMenu(canvas)
    await expect(
      menu.queryByRole("menuitem", { name: "Pin to question" })
    ).toBeNull()
    await userEvent.keyboard("{Escape}")
    await waitFor(() => expect(menu.queryByRole("menu")).toBeNull())
  },
}

/**
 * An external agent receives an approved sample too, through the same gate as
 * a provider (ADR-0034): the pin is offered for it under `sampled`.
 */
export const PinForAnExternalAgent: Story = {
  args: {
    onCopyName: fn(),
    pin: { tier: "sampled", destination: "agent", onPin: fn() },
  },
  play: async ({ canvas }) => {
    const menu = await openInvoicesMenu(canvas)
    await expect(
      menu.getByRole("menuitem", { name: "Pin to question" })
    ).toBeVisible()
    await userEvent.keyboard("{Escape}")
    await waitFor(() => expect(menu.queryByRole("menu")).toBeNull())
  },
}

/** The tree's active row, as a screen reader is told it. */
function activeRow(tree: HTMLElement) {
  const id = tree.getAttribute("aria-activedescendant")
  return id ? document.getElementById(id) : null
}

/**
 * Keyboard only: arrows, Home/End, ← back to the parent, letters to jump.
 * Focus stays on the tree and `aria-activedescendant` names the active row.
 */
export const KeyboardNavigation: Story = {
  play: async ({ canvas, args }) => {
    const tree = canvas.getByRole("tree", { name: "Catalog" })
    tree.focus()
    await userEvent.keyboard("{ArrowRight}")
    await waitFor(() => expect(canvas.getByText("invoices")).toBeVisible())
    await userEvent.keyboard("{ArrowDown}")
    await expect(activeRow(tree)).toHaveTextContent("customers")
    await expect(activeRow(tree)).toHaveAttribute("aria-posinset", "1")
    await expect(activeRow(tree)).toHaveAttribute("aria-setsize", "5")

    await userEvent.keyboard("i")
    await expect(activeRow(tree)).toHaveTextContent("invoices")
    await userEvent.keyboard("i")
    await expect(activeRow(tree)).toHaveTextContent("invoice_lines")

    await userEvent.keyboard("{ArrowLeft}")
    await expect(activeRow(tree)).toHaveTextContent("public")
    await userEvent.keyboard("{End}")
    await expect(activeRow(tree)).toHaveTextContent("pg_catalog")
    await userEvent.keyboard("{Home}")
    await expect(activeRow(tree)).toHaveTextContent("public")
    await expect(tree).toHaveFocus()
    await expect(args.onSelect).not.toHaveBeenCalled()
  },
}

/**
 * A filter removes the focused row: focus lands on a row still shown, and
 * Enter acts on that one — never on the row that disappeared.
 */
export const FocusSurvivesAFilter: Story = {
  play: async ({ canvas, args }) => {
    const tree = canvas.getByRole("tree", { name: "Catalog" })
    tree.focus()
    await userEvent.keyboard("{ArrowRight}{End}")
    await expect(activeRow(tree)).toHaveTextContent("pg_catalog")
    await userEvent.type(canvas.getByLabelText("Filter loaded objects"), "inv")
    await waitFor(() => expect(canvas.queryByText("pg_catalog")).toBeNull())
    const filtered = canvas.getByRole("tree", { name: "Catalog" })
    filtered.focus()
    // Bounded to the last row still shown: `invoice_lines`.
    await expect(activeRow(filtered)).toHaveTextContent("invoice_lines")
    await userEvent.keyboard("{Enter}")
    await expect(args.onSelect).toHaveBeenCalledWith(
      expect.objectContaining({ name: "invoice_lines" })
    )
  },
}

/** 5 000 relations in one schema: only the visible rows are in the DOM. */
export const FiveThousandRelations: Story = {
  args: { nodes: hugeCatalog },
  play: async ({ canvas }) => {
    const tree = canvas.getByRole("tree", { name: "Catalog" })
    tree.focus()
    await userEvent.keyboard("{ArrowRight}")
    await waitFor(() => expect(canvas.getByText("event_0000")).toBeVisible())
    await expect(canvas.getAllByRole("treeitem").length).toBeLessThan(120)
    await userEvent.keyboard("{End}")
    await waitFor(() => expect(activeRow(tree)).toHaveTextContent("event_4999"))
    await expect(activeRow(tree)).toHaveAttribute("aria-setsize", "5000")
  },
}

/** Long, hostile, right-to-left and mixed-script names stay text, read in full on hover. */
export const UnusualNames: Story = {
  args: { nodes: unusualCatalog },
  play: async ({ canvas }) => {
    await userEvent.click(canvas.getByText("public"))
    await expect(canvas.getByText(HOSTILE)).toBeVisible()
    const arabic = canvas.getByText("العملاء")
    await expect(arabic).toHaveAttribute("dir", "auto")
    await expect(arabic.closest("[role=treeitem]")).toHaveAttribute(
      "title",
      "العملاء — جدول العملاء"
    )
    await expect(canvas.getByText("a.b")).toBeVisible()
  },
}

export const Narrow: Story = {
  args: { nodes: unusualCatalog },
  decorators: [
    (Story) => (
      <div className="flex h-[360px] w-[180px] flex-col bg-sidebar p-2">
        <Story />
      </div>
    ),
  ],
  play: async ({ canvas }) => {
    await userEvent.click(canvas.getByText("public"))
    const long = canvas.getByText(/^customer_lifetime_value/)
    await expect(long.scrollWidth).toBeGreaterThan(long.clientWidth)
  },
}

export const RightToLeft: Story = {
  args: { nodes: unusualCatalog },
  decorators: [
    (Story) => (
      <div
        dir="rtl"
        className="flex h-[480px] w-[280px] flex-col bg-sidebar p-2"
      >
        <Story />
      </div>
    ),
  ],
}

export const SearchInProgress: Story = {
  args: { search: { hits: null, searching: true, onQuery: fn() } },
  play: async ({ canvas }) => {
    await userEvent.type(canvas.getByLabelText("Filter loaded objects"), "inv")
    await expect(
      await canvas.findByText("Searching loaded objects…")
    ).toBeVisible()
  },
}
