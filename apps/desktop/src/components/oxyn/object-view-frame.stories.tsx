import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import { ViewIcon } from "@hugeicons/core-free-icons"
import { useHotkeys } from "@tanstack/react-hotkeys"
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
import { PreviewControls } from "./preview-controls"
import { RelationConstraints } from "./relation-constraints"
import { RelationDefinition } from "./relation-definition"
import { RelationIndexes } from "./relation-indexes"
import { IncomingKeys } from "./relation-keys"
import { RelationStructure } from "./relation-structure"
import { ResultPanel } from "./result-panel"
import type { ResultState } from "./result-panel"
import { DropdownMenuItem } from "@/components/ui/dropdown-menu"
import { PLAIN_SHAPE } from "@/lib/ipc/metadata"
import type { ExportFormatChoice } from "@/lib/ipc/results"

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
}: {
  /** The layout of 1200 px and more: the definition sits beside the tabs. */
  wide?: boolean
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
  // `⌘2` as the workspace binds it for the object on screen.
  const [gridFocusRequest, setGridFocusRequest] = React.useState(0)
  const [definitionWidth, setDefinitionWidth] = React.useState<number>(
    DEFINITION_WIDTH.initial
  )
  useHotkeys([
    {
      hotkey: "Mod+2",
      callback: () => {
        if (dataUnavailable !== null) return
        setTab("data")
        setGridFocusRequest((count) => count + 1)
      },
    },
  ])
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
                pagination={{ type: "needsOrder" }}
                onApplyPredicate={() => undefined}
                onApplySort={() => undefined}
                onPage={() => undefined}
                onCancel={onCancel}
                onLoadColumns={() => undefined}
              />
              <div className="min-h-0 flex-1">
                <ResultPanel
                  state={state}
                  fetchPage={syntheticPages(200, 0)}
                  onCancel={onCancel}
                  onRetry={onRefresh}
                  context={{ connectionName: "billing-prod", statement: null }}
                  footerNote="Preview · total row count not requested"
                  footerActions={
                    compact ? null : (
                      <ExportMenuView
                        {...exportChoice}
                        state={idle}
                        onExport={onExport}
                        onCancel={onCancel}
                      />
                    )
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
 * duplicate in the bar (docs/UX-SPEC.md, « Largeur réduite »).
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

/** Inactive sub-tabs against the light palette: axe checks their contrast here. */
export const Light: Story = { globals: { theme: "light" } }
