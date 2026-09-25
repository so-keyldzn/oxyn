import * as React from "react"
import { useHotkeys } from "@tanstack/react-hotkeys"
import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

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
import { PLAIN_SHAPE } from "@/lib/ipc/metadata"

// `Mod` is Command on macOS and Control elsewhere, as the hotkeys decide.
const MOD = /Mac/.test(navigator.platform) ? "Meta" : "Control"

const fetched = {
  state: "fetched",
  fetchedAt: "2026-09-15T09:30:00+00:00",
} as const

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
  onRefresh,
  onCancel,
}: {
  name?: string
  kind?: string
  initial?: ObjectTab
  dataUnavailable?: string | null
  state?: ResultState
  running?: boolean
  cancelling?: boolean
  onRefresh: () => void
  onCancel: () => void
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
  return (
    <ObjectViewFrame
      name={name}
      kind={kind}
      tab={tab}
      onTabChange={setTab}
      dataUnavailable={dataUnavailable}
      onEscape={running ? onCancel : undefined}
      gridFocusRequest={gridFocusRequest}
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
  args: { onRefresh: fn(), onCancel: fn() },
} satisfies Meta<typeof Harness>

export default meta
type Story = StoryObj<typeof meta>

export const Preview: Story = {
  play: async ({ canvas }) => {
    await expect(canvas.getByRole("tab", { name: "Data" })).toHaveAttribute(
      "aria-selected",
      "true"
    )
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

/** Inactive sub-tabs against the light palette: axe checks their contrast here. */
export const Light: Story = { globals: { theme: "light" } }
