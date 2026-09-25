import * as React from "react"
import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

import { CatalogTree, addressKey } from "./catalog-tree"
import { Button } from "@/components/ui/button"
import { catalog } from "./fixtures"
import { centerOf, pressAndMove, release } from "./pointer-drag-fixtures"
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

/**
 * System schemas are hidden from view, never removed from the catalog. The
 * toggle keeps one name and says its state by being pressed: a name that
 * flipped too would be read « Show system objects, pressed ».
 */
export const HideSystemObjects: Story = {
  play: async ({ canvas }) => {
    const toggle = canvas.getByRole("button", { name: "Hide system objects" })
    await expect(canvas.getByText("pg_catalog")).toBeVisible()
    await expect(toggle).toHaveAttribute("aria-pressed", "false")
    await userEvent.click(toggle)
    await expect(toggle).toHaveAttribute("aria-pressed", "true")
    await expect(toggle).toHaveAccessibleName("Hide system objects")
    await expect(canvas.queryByText("pg_catalog")).toBeNull()
    await userEvent.click(toggle)
    await expect(toggle).toHaveAttribute("aria-pressed", "false")
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

/** `public` as the cache leaves it after an eviction: listed, never read. */
const evicted = catalog.map((node, index) =>
  index === 0 ? { ...node, loaded: false, children: [] } : node
)

/**
 * Stands in for the sidebar: « Evict public » publishes the tree the bus
 * leaves after evicting that schema, and reading a level publishes it again.
 */
function EvictionHarness(props: React.ComponentProps<typeof CatalogTree>) {
  const [nodes, setNodes] = React.useState(catalog)
  return (
    <>
      <CatalogTree
        {...props}
        nodes={nodes}
        onExpand={(address) => {
          props.onExpand(address)
          setNodes(catalog)
        }}
      />
      <Button variant="outline" size="sm" onClick={() => setNodes(evicted)}>
        Evict public
      </Button>
    </>
  )
}

/**
 * Expanded, then evicted to keep the catalog cache bounded: the open schema
 * says its contents are not loaded, instead of looking empty, and reads them
 * again only when asked — never on its own, so two schemas that do not fit
 * together cannot evict each other in a loop.
 */
export const ExpandedThenEvicted: Story = {
  render: (args) => <EvictionHarness {...args} />,
  play: async ({ canvas, args }) => {
    await userEvent.click(canvas.getByText("public"))
    await expect(canvas.getByText("invoices")).toBeVisible()

    await userEvent.click(canvas.getByRole("button", { name: "Evict public" }))
    const unloaded = canvas.getByText("Not loaded — click to read")
    await expect(unloaded).toBeVisible()
    await expect(canvas.queryByText("invoices")).toBeNull()
    await expect(args.onExpand).not.toHaveBeenCalled()

    await userEvent.click(unloaded)
    await expect(args.onExpand).toHaveBeenCalledOnce()
    await expect(args.onExpand).toHaveBeenCalledWith(catalog[0]?.address)
    await expect(canvas.getByText("invoices")).toBeVisible()
    await expect(canvas.queryByText("Not loaded — click to read")).toBeNull()
  },
}

/** A level read and legitimately empty: said, not left blank. */
export const EmptySchema: Story = {
  args: {
    nodes: catalog.map((node, index) =>
      index === 1 ? { ...node, loaded: true } : node
    ),
  },
  play: async ({ canvas, args }) => {
    await userEvent.click(canvas.getByText("reporting"))
    await expect(canvas.getByText("No objects")).toBeVisible()
    await expect(args.onExpand).not.toHaveBeenCalled()
  },
}

/** Reading an unloaded level: the contents row says so while it runs. */
export const ReadingUnloadedSchema: Story = {
  args: {
    loading: new Set([addressKey(catalog[1]!.address)]),
    expanded: new Set([addressKey(catalog[1]!.address)]),
  },
  play: async ({ canvas }) => {
    await expect(canvas.getByText("Reading…")).toBeVisible()
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

/**
 * SQLite declares `DDL` without `TRUNCATE`: `Truncate…` is greyed and says
 * why, by capability, never by product name (ADR-0042).
 */
export const OperationsWithoutTruncate: Story = {
  args: {
    onCopyName: fn(),
    operations: {
      capabilities: ["SQL", "DDL", "TRANSACTIONAL_DDL"],
      onOperation: fn(),
    },
  },
  play: async ({ canvas, args }) => {
    const menu = await openInvoicesMenu(canvas)
    const truncate = menu.getByRole("menuitem", { name: /Truncate…/ })
    await expect(truncate).toHaveAttribute("aria-disabled", "true")
    await expect(truncate).toHaveTextContent(
      "This database has no TRUNCATE statement."
    )
    await userEvent.click(menu.getByRole("menuitem", { name: "Drop…" }))
    await expect(args.operations?.onOperation).toHaveBeenCalledWith(
      expect.objectContaining({ name: "invoices" }),
      "drop"
    )
    await waitFor(() => expect(menu.queryByRole("menu")).toBeNull())
  },
}

/** A read-only session declares no `DDL`: the three entries are greyed. */
export const OperationsOnReadOnly: Story = {
  args: {
    onCopyName: fn(),
    operations: { capabilities: ["SQL"], onOperation: fn() },
  },
  play: async ({ canvas }) => {
    const menu = await openInvoicesMenu(canvas)
    for (const name of [/Rename…/, /Truncate…/, /Drop…/]) {
      const item = menu.getByRole("menuitem", { name })
      await expect(item).toHaveAttribute("aria-disabled", "true")
      await expect(item).toHaveTextContent(
        "This connection does not accept schema changes."
      )
    }
    await userEvent.keyboard("{Escape}")
    await waitFor(() => expect(menu.queryByRole("menu")).toBeNull())
  },
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
    // The popup fades in: read it once the transition lets it be seen.
    await waitFor(() =>
      expect(
        menu.getByRole("menuitem", { name: "Pin to question" })
      ).toBeVisible()
    )
    await userEvent.keyboard("{Escape}")
    await waitFor(() => expect(menu.queryByRole("menu")).toBeNull())
  },
}

/**
 * `Copy as ▸` hands the node and the form over: the SQL is composed by the
 * backend, never here (I-10). `DDL` is greyed where the session reads no
 * definition, and says so.
 */
export const CopyAs: Story = {
  args: {
    onCopyName: fn(),
    copyAs: { definition: false, onCopy: fn() },
  },
  play: async ({ canvas, args }) => {
    const menu = await openInvoicesMenu(canvas)
    await userEvent.click(menu.getByRole("menuitem", { name: /Copy as/ }))
    const ddl = await menu.findByRole("menuitem", { name: /^DDL/ })
    await expect(ddl).toHaveAttribute("aria-disabled", "true")
    await expect(ddl).toHaveTextContent(
      "This session does not provide object definitions"
    )
    await userEvent.click(menu.getByRole("menuitem", { name: "SELECT *" }))
    await expect(args.copyAs?.onCopy).toHaveBeenCalledWith(
      expect.objectContaining({ name: "invoices" }),
      "selectAll"
    )
    await waitFor(() => expect(menu.queryByRole("menu")).toBeNull())
  },
}

/**
 * Two relations of the same kind offer the same menu: what an entry depends
 * on is the tree's handlers and the node's kind, never which sibling was
 * right-clicked or whether its level was read.
 */
export const SiblingTablesShareTheirMenu: Story = {
  args: {
    onOpen: fn(),
    onCopyName: fn(),
    onRefresh: fn(),
    copyAs: { definition: true, onCopy: fn() },
    operations: {
      capabilities: ["SQL", "DDL", "TRUNCATE"],
      onOperation: fn(),
    },
  },
  play: async ({ canvas }) => {
    await userEvent.click(canvas.getByText("public"))
    const body = within(document.body)
    const entriesOf = async (name: string) => {
      await userEvent.pointer({
        keys: "[MouseRight]",
        target: canvas.getByText(name),
      })
      await body.findByRole("menuitem", { name: "Copy qualified name" })
      const entries = body
        .getAllByRole("menuitem")
        .map((item) => item.textContent)
      await userEvent.keyboard("{Escape}")
      await waitFor(() => expect(body.queryByRole("menu")).toBeNull())
      return entries
    }
    const customers = await entriesOf("customers")
    await expect(customers).toEqual([
      "Open data",
      "View structure",
      "View DDL",
      "Copy qualified name",
      "Copy as",
      "Collapse all",
      "Rename…",
      "Truncate…",
      "Drop…",
    ])
    await expect(await entriesOf("invoices")).toEqual(customers)
  },
}

/** The hint of the row under the pointer, wherever it renders. */
function rowHint() {
  return document.querySelector('[data-slot="tooltip-content"]')
}

/**
 * A row's full name shows when the pointer rests on it, and closes when its
 * context menu opens: a hint left over the menu hides its first entries.
 */
export const HintClosesOnMenu: Story = {
  args: { onOpen: fn(), onCopyName: fn() },
  play: async ({ canvas }) => {
    await userEvent.click(canvas.getByText("public"))
    const row = canvas.getByText("invoices")
    await userEvent.hover(row)
    await waitFor(() => expect(rowHint()).toHaveTextContent("invoices"), {
      timeout: 2000,
    })

    await userEvent.pointer({ keys: "[MouseRight]", target: row })
    const menu = within(document.body)
    await menu.findByRole("menuitem", { name: "Copy qualified name" })
    await waitFor(() => expect(rowHint()).toBeNull())
    // Resting on the row while the menu is open does not bring it back.
    await userEvent.hover(row)
    await new Promise((resolve) => window.setTimeout(resolve, 800))
    await expect(rowHint()).toBeNull()
    await userEvent.keyboard("{Escape}")
    await waitFor(() => expect(menu.queryByRole("menu")).toBeNull())
  },
}

/**
 * On a schema, where a console's session context can be one: `New console on
 * this schema` hands the schema over, and nothing runs. It is absent from a
 * table's menu.
 */
export const NewConsoleOnThisSchema: Story = {
  args: { onCopyName: fn(), onRefresh: fn(), onNewConsole: fn() },
  play: async ({ canvas, args }) => {
    const menu = await openInvoicesMenu(canvas)
    await expect(
      menu.queryByRole("menuitem", { name: "New console on this schema" })
    ).toBeNull()
    await userEvent.keyboard("{Escape}")
    await waitFor(() => expect(menu.queryByRole("menu")).toBeNull())

    await userEvent.pointer({
      keys: "[MouseRight]",
      target: canvas.getByText("public"),
    })
    await userEvent.click(
      await menu.findByRole("menuitem", { name: "New console on this schema" })
    )
    await expect(args.onNewConsole).toHaveBeenCalledWith(
      expect.objectContaining({ name: "public", kind: "namespace" })
    )
    await waitFor(() => expect(menu.queryByRole("menu")).toBeNull())
  },
}

/** `Collapse all` folds every level; with nothing open it says so. */
export const CollapseAll: Story = {
  args: { onRefresh: fn() },
  play: async ({ canvas }) => {
    await userEvent.click(canvas.getByText("public"))
    await expect(canvas.getByText("invoices")).toBeVisible()
    const menu = within(document.body)
    await userEvent.pointer({
      keys: "[MouseRight]",
      target: canvas.getByText("public"),
    })
    await userEvent.click(
      await menu.findByRole("menuitem", { name: "Collapse all" })
    )
    await waitFor(() => expect(canvas.queryByText("invoices")).toBeNull())
    await userEvent.pointer({
      keys: "[MouseRight]",
      target: canvas.getByText("public"),
    })
    const collapse = await menu.findByRole("menuitem", { name: /Collapse all/ })
    await expect(collapse).toHaveAttribute("aria-disabled", "true")
    await expect(collapse).toHaveTextContent("Nothing is expanded")
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
    // The comment is the row's description, and its hint on hover.
    const row = arabic.closest("[role=treeitem]")
    await expect(row).toHaveAccessibleDescription("جدول العملاء")
    if (row) await userEvent.hover(row)
    await waitFor(
      () => expect(rowHint()).toHaveTextContent("العملاء — جدول العملاء"),
      { timeout: 2000 }
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

/**
 * A relation dragged out of the tree carries its name under the pointer, and
 * the parent is told where it was released; a schema does not move.
 */
export const DragARelationName: Story = {
  args: { nameTransfer: { onDrop: fn(), onInsert: fn() } },
  play: async ({ canvas, args }) => {
    await userEvent.click(canvas.getByText("public"))
    const invoices = canvas.getByText("invoices")
    const from = centerOf(invoices)
    const to = { x: from.x + 160, y: from.y + 40 }
    pressAndMove(invoices, to)
    const ghost = await waitFor(() => {
      const found = document.querySelector('[data-slot="catalog-drag-ghost"]')
      expect(found).toHaveTextContent("invoices")
      return found
    })
    await expect(ghost).toHaveAttribute("aria-hidden", "true")
    release(invoices, to)
    await expect(args.nameTransfer?.onDrop).toHaveBeenCalledWith(
      expect.objectContaining({ name: "invoices" }),
      to
    )
    await waitFor(() =>
      expect(
        document.querySelector('[data-slot="catalog-drag-ghost"]')
      ).toBeNull()
    )

    const schema = canvas.getByText("public")
    const at = centerOf(schema)
    pressAndMove(schema, { x: at.x + 160, y: at.y })
    release(schema, { x: at.x + 160, y: at.y })
    await expect(args.nameTransfer?.onDrop).toHaveBeenCalledTimes(1)
  },
}

/** Esc drops nothing: the drag ends where it started. */
export const DragCancelledByEscape: Story = {
  args: { nameTransfer: { onDrop: fn(), onInsert: fn() } },
  play: async ({ canvas, args }) => {
    await userEvent.click(canvas.getByText("public"))
    const invoices = canvas.getByText("invoices")
    const to = { x: centerOf(invoices).x + 120, y: centerOf(invoices).y }
    pressAndMove(invoices, to)
    await userEvent.keyboard("{Escape}")
    release(invoices, to)
    await expect(args.nameTransfer?.onDrop).not.toHaveBeenCalled()
  },
}

/**
 * ⌥↵, the keyboard's side of the drag: the focused relation's name goes to
 * the console. The row is not opened, and a hostile name is handed over as
 * the node it is — its quoting is the backend's, never the tree's (I-10).
 */
export const InsertANameFromTheKeyboard: Story = {
  args: {
    nodes: unusualCatalog,
    nameTransfer: { onDrop: fn(), onInsert: fn() },
  },
  play: async ({ canvas, args }) => {
    const tree = canvas.getByRole("tree")
    await expect(tree).toHaveAttribute("aria-keyshortcuts", "Alt+Enter")
    await userEvent.click(canvas.getByText("public"))
    await userEvent.click(canvas.getByText(HOSTILE))
    args.onSelect.mockClear()
    tree.focus()
    await userEvent.keyboard("{Alt>}{Enter}{/Alt}")
    await expect(args.nameTransfer?.onInsert).toHaveBeenCalledWith(
      expect.objectContaining({ name: HOSTILE })
    )
    await expect(args.onSelect).not.toHaveBeenCalled()
  },
}
