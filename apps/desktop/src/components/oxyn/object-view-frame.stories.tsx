import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import { ViewIcon } from "@hugeicons/core-free-icons"
import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

import { DEFINITION_WIDTH } from "./definition-beside"
import { ExportMenuView, ExportSubmenu } from "./export-menu"
import { FacetFrame } from "./facet-frame"
import { invoiceColumns, invoicesDetail, syntheticPages } from "./fixtures"
import {
  HOSTILE,
  incomingKeys,
  invoiceConstraints,
  invoiceIndexes,
  invoicesDefinition,
} from "./metadata-fixtures"
import {
  ObjectViewFrame,
  PreviewToolbar,
  RelationsPanel,
  initialObjectTab,
} from "./object-view-frame"
import type { ObjectTab } from "./object-view-frame"
import {
  PreviewControls,
  SortToBrowsePages,
  previewFooterNote,
} from "./preview-controls"
import { RelationConstraints } from "./relation-constraints"
import { RelationDefinition } from "./relation-definition"
import { RelationIndexes } from "./relation-indexes"
import { IncomingKeys } from "./relation-keys"
import { RelationStructure } from "./relation-structure"
import { ResultPanel } from "./result-panel"
import type { ResultState } from "./result-panel"
import { DropdownMenuItem } from "@/components/ui/dropdown-menu"
import { PLAIN_SHAPE } from "@/lib/ipc/metadata"
import type { Pagination } from "@/lib/ipc/metadata"
import type { ExportFormatChoice } from "@/lib/ipc/results"
import { useActionSource } from "@/lib/actions/context"
import type { WorkspaceActions } from "@/lib/actions/context"

const nothing = () => undefined
const storyWorkspace: WorkspaceActions = {
  openConsole: nothing,
  closeActiveTab: nothing,
  nextTab: nothing,
  previousTab: nothing,
  showCatalog: nothing,
  showLibrary: nothing,
  focusPreview: nothing,
  focusConsole: nothing,
  toggleSidebar: nothing,
  toggleAside: nothing,
  openAssistant: nothing,
  switchConnection: nothing,
}

// `Mod` is Command on macOS and Control elsewhere, as the hotkeys decide.
const MOD = /Mac/.test(navigator.platform) ? "Meta" : "Control"

const fetched = {
  state: "fetched",
  fetchedAt: "2026-09-15T09:30:00+00:00",
} as const

const exportFormats: Array<ExportFormatChoice> = [
  { format: "csv", label: "CSV", extension: "csv", supported: true },
  { format: "json", label: "JSON", extension: "json", supported: true },
]

const populated: ResultState = {
  status: "populated",
  result: "story-preview",
  columns: invoiceColumns,
  rows: 200,
  complete: true,
  truncated: false,
  cancelled: false,
  elapsedMs: 42,
}

/** A table read in its primary key's order: pages from the first read. */
const keyOrder: Pagination = {
  type: "ready",
  previous: false,
  next: true,
  firstRow: 1,
  orderedBy: ["id"],
}

/** A whole object view, drawn from fixtures: no backend is reached. */
function Harness({
  name = "invoices",
  kind = "table",
  initial,
  dataUnavailable = null,
  state = populated,
  running = false,
  cancelling = false,
  compact = false,
  onRefresh,
  onCancel,
  wide = false,
  onExport,
  onInspectRow,
  absentTabs,
  pagination = keyOrder,
}: {
  /** The layout of 1200 px and more: the definition sits beside the tabs. */
  wide?: boolean
  pagination?: Pagination
  absentTabs?: ReadonlySet<ObjectTab>
  name?: string
  kind?: string
  initial?: ObjectTab
  dataUnavailable?: string | null
  state?: ResultState
  running?: boolean
  cancelling?: boolean
  compact?: boolean
  onRefresh: () => void
  onCancel: () => void
  onExport: () => void
  onInspectRow: () => void
}) {
  const [tab, setTab] = React.useState<ObjectTab>(
    initial ?? initialObjectTab(undefined, dataUnavailable === null)
  )
  const [direction, setDirection] = React.useState<"incoming" | "outgoing">(
    "incoming"
  )
  const idle = { status: "idle" } as const
  const [sortOpen, setSortOpen] = React.useState(false)
  // `⌘2` as the workspace publishes it to the registry for the object on
  // screen; the rest of the workspace does nothing in a story.
  const [gridFocusRequest, setGridFocusRequest] = React.useState(0)
  const [definitionWidth, setDefinitionWidth] = React.useState<number>(
    DEFINITION_WIDTH.initial
  )
  useActionSource(
    "workspace",
    {
      activeTab: "object",
      tabCount: 1,
      consoleCount: 0,
      objectActive: true,
      hasAside: false,
      hasAssistant: false,
    },
    {
      ...storyWorkspace,
      focusPreview: () => {
        if (dataUnavailable !== null) return
        setTab("data")
        setGridFocusRequest((count) => count + 1)
      },
    }
  )
  const exportChoice = {
    formats: exportFormats,
    formatsFailed: false,
    exportable: state.status === "populated",
    reason: "Export becomes available once the preview has finished loading.",
    label: "Export preview…",
    scope: "The preview rows shown · not the entire table",
  }
  return (
    <ObjectViewFrame
      name={name}
      kind={kind}
      tab={tab}
      onTabChange={setTab}
      compact={compact}
      absentTabs={absentTabs}
      dataUnavailable={dataUnavailable}
      onEscape={running ? onCancel : undefined}
      gridFocusRequest={gridFocusRequest}
      definitionLayout={{
        beside: wide,
        width: definitionWidth,
        onWidthChange: setDefinitionWidth,
      }}
      toolbar={
        tab === "data" && dataUnavailable === null ? (
          <PreviewToolbar
            running={running}
            cancelling={cancelling}
            onRefresh={onRefresh}
            onCancel={onCancel}
          />
        ) : null
      }
      panels={{
        data:
          dataUnavailable === null ? (
            <>
              <PreviewControls
                canFilter
                canSort
                columns={invoicesDetail.fields.map((field) => field.name)}
                applied={PLAIN_SHAPE}
                status={running ? "loading" : "loaded"}
                pagination={pagination}
                onApplyPredicate={() => undefined}
                onApplySort={() => undefined}
                onPage={() => undefined}
                onCancel={onCancel}
                onLoadColumns={() => undefined}
                sortOpen={sortOpen}
                onSortOpenChange={setSortOpen}
              />
              <div className="min-h-0 flex-1">
                <ResultPanel
                  state={state}
                  fetchPage={syntheticPages(200, 0)}
                  onCancel={onCancel}
                  onRetry={onRefresh}
                  context={{ connectionName: "billing-prod", statement: null }}
                  footerNote={previewFooterNote(
                    pagination,
                    state.status === "populated" ? state.rows : 0
                  )}
                  footerActions={
                    <>
                      {pagination.type === "needsOrder" ? (
                        <SortToBrowsePages onClick={() => setSortOpen(true)} />
                      ) : null}
                      {compact ? null : (
                        <ExportMenuView
                          {...exportChoice}
                          state={idle}
                          onExport={onExport}
                          onCancel={onCancel}
                        />
                      )}
                    </>
                  }
                  compactActions={
                    compact ? (
                      <>
                        <ExportSubmenu
                          {...exportChoice}
                          exporting={false}
                          onExport={onExport}
                        />
                        <DropdownMenuItem onClick={onInspectRow}>
                          <HugeiconsIcon icon={ViewIcon} strokeWidth={2} />
                          Inspect row
                        </DropdownMenuItem>
                      </>
                    ) : undefined
                  }
                />
              </div>
            </>
          ) : null,
        structure: (
          <FacetFrame
            label="columns"
            freshness={fetched}
            load={idle}
            unsupported={null}
            hasValue
            empty={false}
            emptyText=""
            onRefresh={onRefresh}
            onCancel={onCancel}
          >
            <RelationStructure detail={invoicesDetail} />
          </FacetFrame>
        ),
        indexes: <RelationIndexes indexes={invoiceIndexes} />,
        constraints: (
          <RelationConstraints
            constraints={invoiceConstraints}
            detail={invoicesDetail}
            indexes={invoiceIndexes}
            onCopyDefinition={() => undefined}
          />
        ),
        relations: (
          <RelationsPanel
            direction={direction}
            onDirectionChange={setDirection}
          >
            <IncomingKeys keys={incomingKeys} />
          </RelationsPanel>
        ),
        definition: (
          <RelationDefinition definition={invoicesDefinition} stale={false} />
        ),
      }}
    />
  )
}

const meta = {
  title: "Oxyn/ObjectView",
  component: Harness,
  decorators: [
    (Story) => (
      <div className="h-[560px] w-full border">
        <Story />
      </div>
    ),
  ],
  args: {
    onRefresh: fn(),
    onCancel: fn(),
    onExport: fn(),
    onInspectRow: fn(),
  },
} satisfies Meta<typeof Harness>

export default meta
type Story = StoryObj<typeof meta>

/** Wide: the six sub-tabs, and the preview's actions each in the bar. */
export const Preview: Story = {
  play: async ({ canvas }) => {
    await expect(canvas.getByRole("tab", { name: "Data" })).toHaveAttribute(
      "aria-selected",
      "true"
    )
    await expect(canvas.getAllByRole("tab")).toHaveLength(6)
    await expect(canvas.queryByRole("button", { name: /^More/ })).toBeNull()
    await expect(canvas.getByRole("button", { name: "Columns" })).toBeVisible()
    await expect(
      canvas.getByRole("button", { name: "Export preview…" })
    ).toBeVisible()
    await expect(canvas.queryByRole("button", { name: "Actions" })).toBeNull()
    await expect(canvas.getByText("Read-only preview")).toBeVisible()
    // Refresh and Cancel are two buttons: the idle one is inert.
    await expect(canvas.getByRole("button", { name: /^Cancel/ })).toBeDisabled()
    // The key order gives pages from the first read, with no sort (ADR-0068).
    await expect(
      canvas.getByRole("button", { name: /Next page/ })
    ).toBeEnabled()
    await expect(
      canvas.getByText(/in the order of the primary key \(id\)/)
    ).toBeVisible()
  },
}

/** A later page of the key order: the footer says which rows these are. */
export const PreviewSecondPage: Story = {
  args: {
    pagination: { ...keyOrder, previous: true, firstRow: 201 },
  },
  play: async ({ canvas }) => {
    await expect(
      canvas.getByText("Preview · rows 201–400 · total row count not requested")
    ).toBeVisible()
  },
}

/**
 * A relation with no default order: the footer, where a pager is looked for,
 * says how to get one and opens the Sort menu.
 */
export const PreviewWithoutOrder: Story = {
  args: { pagination: { type: "needsOrder" } },
  play: async ({ canvas }) => {
    await expect(canvas.queryByRole("button", { name: /Next page/ })).toBeNull()
    await expect(
      canvas.getByText("Preview · total row count not requested")
    ).toBeVisible()
    await userEvent.click(
      canvas.getByRole("button", { name: "Sort to browse pages" })
    )
    const popover = within(document.body)
    await expect(
      await popover.findByRole("button", { name: "Add column" })
    ).toBeVisible()
    await userEvent.keyboard("{Escape}")
    await waitFor(() =>
      expect(popover.queryByRole("button", { name: "Add column" })).toBeNull()
    )
    await waitFor(() =>
      expect(document.querySelector("[data-base-ui-focus-guard]")).toBeNull()
    )
  },
}

/**
 * Editing is not offered in a read-only preview: the control is disabled, and
 * why is readable from the keyboard through the help trigger beside it.
 */
export const EditRowsUnavailable: Story = {
  play: async ({ canvas }) => {
    const edit = canvas.getByRole("button", { name: "Edit rows…" })
    await expect(edit).toBeDisabled()
    await expect(edit).toHaveAccessibleDescription(
      /needs an editable view, a session that allows writes/
    )
    const help = canvas.getByRole("button", { name: "Why unavailable?" })
    await expect(help).toBeEnabled()
    help.focus()
    await userEvent.keyboard("{Shift>}{Tab}{/Shift}{Tab}")
    await expect(help).toHaveFocus()
    // Focus alone shows the reason, without a pointer.
    await waitFor(
      () =>
        expect(
          document.querySelector('[data-slot="tooltip-content"]')
        ).toHaveTextContent(/editable view.*allows writes.*capabilities/),
      { timeout: 2000 }
    )
  },
}

/** `⌘2` shows Data and puts the keyboard in its grid, from any tab. */
export const FocusGridShortcut: Story = {
  args: { initial: "structure" },
  play: async ({ canvas }) => {
    canvas.getByRole("tab", { name: "Structure" }).focus()
    await userEvent.keyboard(`{${MOD}>}2{/${MOD}}`)
    await waitFor(() =>
      expect(canvas.getByRole("tab", { name: "Data" })).toHaveAttribute(
        "aria-selected",
        "true"
      )
    )
    await waitFor(() => expect(canvas.getByRole("grid")).toHaveFocus())
  },
}

/** Without a preview, `⌘2` changes nothing. */
export const FocusGridShortcutWithoutPreview: Story = {
  args: {
    kind: "view",
    dataUnavailable: "This session cannot read rows with a query.",
  },
  play: async ({ canvas }) => {
    const structure = canvas.getByRole("tab", { name: "Structure" })
    structure.focus()
    await userEvent.keyboard(`{${MOD}>}2{/${MOD}}`)
    await expect(structure).toHaveAttribute("aria-selected", "true")
    await expect(structure).toHaveFocus()
  },
}

/** Esc cancels a running preview; Refresh cannot be pressed meanwhile. */
export const PreviewRunning: Story = {
  args: {
    running: true,
    state: { status: "running", rows: 0, serverCancel: true },
  },
  play: async ({ canvas, args }) => {
    await expect(
      canvas.getByRole("button", { name: "Refresh data" })
    ).toBeDisabled()
    canvas.getByRole("tab", { name: "Data" }).focus()
    await userEvent.keyboard("{Escape}")
    await expect(args.onCancel).toHaveBeenCalledOnce()
  },
}

export const PreviewCancelling: Story = {
  args: {
    running: true,
    cancelling: true,
    state: { status: "running", rows: 1200, serverCancel: true },
  },
  play: async ({ canvas }) => {
    await expect(
      canvas.getByRole("button", { name: /Cancelling/ })
    ).toBeDisabled()
  },
}

export const PreviewRefused: Story = {
  args: {
    state: {
      status: "error",
      message:
        'ERROR:  syntax error at or near "><"\nLINE 1: ... WHERE id >< 3\nSQLSTATE: 42601',
      retryable: false,
    },
  },
}

/**
 * A view without rows: Data is disabled, never the selected tab, and its
 * reason is readable.
 */
export const NoRowsToPreview: Story = {
  args: {
    name: "active_customers",
    kind: "view",
    dataUnavailable: "This session cannot read rows with a query.",
  },
  play: async ({ canvas }) => {
    const data = canvas.getByRole("tab", { name: "Data" })
    await expect(data).toHaveAttribute("aria-disabled", "true")
    await expect(
      canvas.getByRole("tab", { name: "Structure" })
    ).toHaveAttribute("aria-selected", "true")
    await expect(data).toHaveAccessibleDescription(
      /cannot read rows with a query/
    )
  },
}

/** Tabs move with the arrows, as Base UI tabs do. */
export const KeyboardOnly: Story = {
  play: async ({ canvas }) => {
    canvas.getByRole("tab", { name: "Data" }).focus()
    await userEvent.keyboard("{ArrowRight}{Enter}")
    await waitFor(() =>
      expect(canvas.getByRole("tab", { name: "Structure" })).toHaveAttribute(
        "aria-selected",
        "true"
      )
    )
    await userEvent.keyboard("{End}{Enter}")
    await waitFor(() =>
      expect(canvas.getByRole("tab", { name: "DDL" })).toHaveAttribute(
        "aria-selected",
        "true"
      )
    )
    await expect(canvas.getByLabelText("Object definition")).toBeVisible()
  },
}

export const HostileName: Story = {
  args: { name: HOSTILE, initial: "structure" },
  play: async ({ canvas }) => {
    const heading = canvas.getByRole("heading", { level: 2 })
    await expect(heading).toHaveTextContent(HOSTILE)
    await expect(heading.querySelector("*")).toBeNull()
  },
}

export const RightToLeftName: Story = {
  args: { name: "חשבוניות_לקוחות_ארכיון_2026", initial: "relations" },
  play: async ({ canvas }) => {
    await expect(canvas.getByRole("heading", { level: 2 })).toHaveAttribute(
      "dir",
      "auto"
    )
    const panel = canvas.getByRole("tabpanel")
    await expect(within(panel).getByText("Many to one")).toBeVisible()
  },
}

export const VeryLongName: Story = {
  args: {
    name: "customer_invoice_line_items_with_tax_breakdown_archived_before_the_2019_migration",
    initial: "indexes",
  },
}

/** A narrow window: the tab bar wraps, nothing overflows the page. */
export const Narrow: Story = {
  decorators: [
    (Story) => (
      <div className="h-[560px] w-[420px] border">
        <Story />
      </div>
    ),
  ],
  play: async ({ canvasElement }) => {
    const frame = canvasElement.firstElementChild as HTMLElement
    await expect(frame.scrollWidth).toBeLessThanOrEqual(frame.clientWidth + 1)
  },
}

/**
 * A table that cannot be previewed in this session: Data is disabled from the
 * first render, and no preview toolbar is drawn.
 */
export const TableNotPreviewable: Story = {
  args: {
    name: "invoices",
    kind: "table",
    dataUnavailable: "This session cannot read rows with a query.",
  },
  play: async ({ canvas }) => {
    await expect(canvas.getByRole("tab", { name: "Data" })).toHaveAttribute(
      "aria-disabled",
      "true"
    )
    await expect(canvas.queryByText("Read-only preview")).toBeNull()
    await expect(
      canvas.queryByRole("button", { name: "Refresh data" })
    ).toBeNull()
  },
}

/**
 * Opened on Structure: the Data panel is not visible on the first render, and
 * appears whole when its tab is chosen.
 */
export const DataNotVisibleAtFirst: Story = {
  args: { initial: "structure" },
  play: async ({ canvas }) => {
    await expect(canvas.queryByText("Read-only preview")).toBeNull()
    await userEvent.click(canvas.getByRole("tab", { name: "Data" }))
    await waitFor(() =>
      expect(canvas.getByText("Read-only preview")).toBeVisible()
    )
    await expect(
      canvas.getByRole("button", { name: "Refresh data" })
    ).toBeEnabled()
  },
}

const wideFrame = (Story: React.ComponentType) => (
  <div className="h-[560px] w-[1280px] border">
    <Story />
  </div>
)

const widthOf = (element: HTMLElement) =>
  Math.round(element.getBoundingClientRect().width)

/**
 * The wide layout: the definition accompanies Structure instead of being a
 * tab, at 424 px; its handle moves by keyboard within 320–640 px, and Home
 * restores 424 px.
 */
export const DefinitionBeside: Story = {
  args: { wide: true, initial: "structure" },
  decorators: [wideFrame],
  play: async ({ canvas }) => {
    await expect(canvas.queryByRole("tab", { name: "DDL" })).toBeNull()
    const panel = canvas.getByRole("region", { name: "Definition" })
    await expect(
      within(panel).getByLabelText("Object definition")
    ).toBeVisible()
    await expect(widthOf(panel)).toBe(DEFINITION_WIDTH.initial)

    const handle = canvas.getByRole("separator", {
      name: "Resize the definition panel",
    })
    handle.focus()
    await userEvent.keyboard("{ArrowLeft}")
    await waitFor(() =>
      expect(widthOf(panel)).toBeGreaterThan(DEFINITION_WIDTH.initial)
    )
    for (let step = 0; step < 12; step += 1)
      await userEvent.keyboard("{ArrowLeft}")
    await waitFor(() => expect(widthOf(panel)).toBe(DEFINITION_WIDTH.max))
    for (let step = 0; step < 12; step += 1)
      await userEvent.keyboard("{ArrowRight}")
    await waitFor(() => expect(widthOf(panel)).toBe(DEFINITION_WIDTH.min))
    await userEvent.keyboard("{Home}")
    await waitFor(() => expect(widthOf(panel)).toBe(DEFINITION_WIDTH.initial))
  },
}

/**
 * The width is the workspace's: leaving for Data drops the panel, coming back
 * to another metadata tab draws it again at the width it was left at.
 */
export const DefinitionBesideKeepsItsWidth: Story = {
  args: { wide: true, initial: "structure" },
  decorators: [wideFrame],
  play: async ({ canvas }) => {
    canvas
      .getByRole("separator", { name: "Resize the definition panel" })
      .focus()
    await userEvent.keyboard("{End}")
    const narrowed = () => canvas.getByRole("region", { name: "Definition" })
    await waitFor(() => expect(widthOf(narrowed())).toBe(DEFINITION_WIDTH.min))

    await userEvent.click(canvas.getByRole("tab", { name: "Data" }))
    await waitFor(() =>
      expect(canvas.queryByRole("region", { name: "Definition" })).toBeNull()
    )
    await userEvent.click(canvas.getByRole("tab", { name: "Indexes" }))
    await waitFor(() => expect(widthOf(narrowed())).toBe(DEFINITION_WIDTH.min))
  },
}

/**
 * Below 1200 px: Data and Structure stay tabs, the four others are in `More`;
 * Columns, Export preview… and Inspect row are in `Actions` only, with no
 * duplicate in the bar (docs/UX-SPEC.md, « Reduced width »).
 */
export const Compact: Story = {
  args: { compact: true },
  decorators: [
    (Story) => (
      <div className="h-[560px] w-[900px] border">
        <Story />
      </div>
    ),
  ],
  play: async ({ canvas, args }) => {
    const body = within(document.body)
    await expect(
      canvas.getAllByRole("tab").map((tab) => tab.textContent)
    ).toEqual(["Data", "Structure"])
    await expect(canvas.getByText("Read-only preview")).toBeVisible()
    await expect(canvas.queryByRole("button", { name: "Columns" })).toBeNull()
    await expect(
      canvas.queryByRole("button", { name: "Export preview…" })
    ).toBeNull()

    await userEvent.click(canvas.getByRole("button", { name: "Actions" }))
    await waitFor(() =>
      expect(body.getByRole("menuitem", { name: /Columns/ })).toBeVisible()
    )
    await expect(
      body.getByRole("menuitem", { name: "Export preview…" })
    ).toBeVisible()
    await userEvent.click(body.getByRole("menuitem", { name: "Inspect row" }))
    await expect(args.onInspectRow).toHaveBeenCalled()
    await waitFor(() => expect(body.queryByRole("menu")).toBeNull())

    await userEvent.click(canvas.getByRole("button", { name: "More" }))
    await userEvent.click(
      await body.findByRole("menuitemradio", { name: "DDL" })
    )
    await waitFor(() =>
      expect(canvas.getByRole("button", { name: "More · DDL" })).toBeVisible()
    )
    await expect(canvas.getByLabelText("Object definition")).toBeVisible()
    await expect(canvas.queryByText("Read-only preview")).toBeNull()

    // Back to a tab of the bar: `More` names nothing again.
    await userEvent.click(canvas.getByRole("tab", { name: "Structure" }))
    await waitFor(() =>
      expect(canvas.getByRole("button", { name: "More" })).toBeVisible()
    )
  },
}

/** What a key-value session cannot fill: no index, constraint or key. */
const KEY_VALUE_ABSENT: ReadonlySet<ObjectTab> = new Set<ObjectTab>([
  "indexes",
  "constraints",
  "relations",
])

/**
 * A key pattern of a key-value store: only the tabs its session can fill,
 * and its kind written for reading, not as the wire name.
 */
export const KeyPatternOfAKeyValueStore: Story = {
  args: {
    name: "session:*",
    kind: "key_pattern",
    absentTabs: KEY_VALUE_ABSENT,
  },
  play: async ({ canvas }) => {
    await expect(
      canvas.getAllByRole("tab").map((tab) => tab.textContent)
    ).toEqual(["Data", "Structure", "DDL"])
    await expect(canvas.getByText("key pattern")).toBeVisible()
    await expect(canvas.queryByText("key_pattern")).toBeNull()
  },
}

/** Narrow, the same session: `More` offers DDL alone. */
export const KeyPatternCompact: Story = {
  args: {
    name: "session:*",
    kind: "key_pattern",
    absentTabs: KEY_VALUE_ABSENT,
    compact: true,
  },
  decorators: [
    (Story) => (
      <div className="h-[560px] w-[900px] border">
        <Story />
      </div>
    ),
  ],
  play: async ({ canvas }) => {
    const body = within(document.body)
    await userEvent.click(canvas.getByRole("button", { name: "More" }))
    await waitFor(() =>
      expect(
        body.getAllByRole("menuitemradio").map((item) => item.textContent)
      ).toEqual(["DDL"])
    )
    // Closed before axe runs: an open menu leaves its focus guards behind.
    await userEvent.keyboard("{Escape}")
    await waitFor(() => expect(body.queryByRole("menu")).toBeNull())
  },
}

/** Inactive sub-tabs against the light palette: axe checks their contrast here. */
export const Light: Story = { globals: { theme: "light" } }
