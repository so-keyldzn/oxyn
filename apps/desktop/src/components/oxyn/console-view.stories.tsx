import * as React from "react"
import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor } from "storybook/test"

import { ConsoleToolbar } from "./console-toolbar"
import { ConsoleView, MIN_VISIBLE_LINES } from "./console-view"
import { invoiceColumns, syntheticPages } from "./fixtures"
import { OfflineConsoleBar } from "./offline-console-bar"
import { ParameterEditor } from "./parameter-editor"
import { ResultFindBar } from "./result-find-bar"
import { HEADER_HEIGHT } from "./result-grid"
import { ResultPanel } from "./result-panel"
import type { ResultState } from "./result-panel"
import { SqlEditor } from "./sql-editor"

const SQL =
  "SELECT c.name, i.amount\nFROM invoices AS i\nJOIN customers AS c ON c.id = i.customer_id\nWHERE i.paid_at IS NULL AND i.amount > $1;"

type ToolbarProps = React.ComponentProps<typeof ConsoleToolbar>

const toolbarDefaults: ToolbarProps = {
  running: false,
  canRun: true,
  readOnly: false,
  target: "statement",
  onRun: fn(),
  onRunAll: fn(),
  onExplain: fn(),
  onCancel: fn(),
  parameterCount: 0,
  parametersOpen: false,
  onToggleParameters: fn(),
  title: "unpaid invoices.sql",
  onTitleChange: fn(),
  titleError: null,
  save: { status: "idle", notice: "Saved." },
  onSave: fn(),
  onSaveAsNew: fn(),
}

/** One console, every part given as the container would give it. */
function consoleParts({
  toolbar = {},
  result = { status: "initial" },
  parameters,
  notice = null,
  text = SQL,
  results,
}: {
  toolbar?: Partial<ToolbarProps>
  result?: ResultState
  parameters?: React.ReactNode
  notice?: string | null
  text?: string
  /** Replaces the result area entirely, for the streaming stories. */
  results?: React.ReactNode
}): React.ComponentProps<typeof ConsoleView> {
  const bar = { ...toolbarDefaults, ...toolbar }
  return {
    toolbar: <ConsoleToolbar {...bar} />,
    notice,
    draftNotice: "Draft saved locally",
    parameters,
    editor: (
      <SqlEditor
        value={text}
        onChange={fn()}
        driver="postgres"
        running={bar.running}
        readOnly={bar.readOnly && !bar.canRun}
        onRun={fn()}
      />
    ),
    results: results ?? (
      <ResultPanel
        state={result}
        fetchPage={syntheticPages(250_000)}
        onCancel={bar.onCancel}
        onRetry={fn()}
      />
    ),
  }
}

const meta = {
  title: "Oxyn/ConsoleView",
  component: ConsoleView,
  decorators: [
    (Story) => (
      <div className="h-[640px] w-[1100px] border">
        <Story />
      </div>
    ),
  ],
  args: consoleParts({}),
} satisfies Meta<typeof ConsoleView>

export default meta
type Story = StoryObj<typeof meta>

export const Initial: Story = {
  play: async ({ canvas }) => {
    await expect(canvas.getByText("No result yet")).toBeVisible()
    await expect(canvas.getByText("Draft saved locally")).toBeVisible()
  },
}

/**
 * A restored working copy before any connection: the text and the name stay
 * editable and saved locally, Run stays off, and connecting is its own button.
 */
export const Offline: Story = {
  args: {
    ...consoleParts({
      toolbar: { canRun: false },
      notice:
        "Recovered offline. Choose a connection before running. Nothing was executed.",
    }),
    offline: (
      <OfflineConsoleBar
        connectionName="billing-prod"
        sameConnection
        attaching={false}
        onAttach={fn()}
        onCancel={fn()}
      />
    ),
  },
  play: async ({ canvas }) => {
    await expect(canvas.getByText("Offline")).toBeVisible()
    await expect(canvas.getByRole("button", { name: /^Run$/ })).toBeDisabled()
    await expect(
      canvas.getByRole("button", { name: "Connect to billing-prod" })
    ).toBeEnabled()
    await expect(canvas.getByRole("textbox", { name: /name/i })).toBeEnabled()
  },
}

export const Running: Story = {
  args: consoleParts({
    toolbar: { running: true, elapsedMs: 4_200 },
    result: { status: "running", rows: 18_000, serverCancel: true },
  }),
  play: async ({ canvas }) => {
    await expect(canvas.getByRole("button", { name: /^Run$/ })).toBeDisabled()
    await expect(canvas.getByText(/received so far/)).toBeVisible()
  },
}

export const Cancelling: Story = {
  args: consoleParts({
    toolbar: { running: true, cancelling: true, elapsedMs: 61_000 },
    result: { status: "running", rows: 0, serverCancel: true },
  }),
}

export const Populated: Story = {
  args: consoleParts({
    notice: "Returned 250,000 rows in 1.4 s.",
    result: {
      status: "populated",
      result: "console-view-populated",
      columns: invoiceColumns,
      rows: 250_000,
      complete: true,
      truncated: false,
      cancelled: false,
    },
  }),
}

/** Data rows whose whole height sits inside the grid's viewport, with text. */
function readableRows(grid: HTMLElement) {
  const box = grid.getBoundingClientRect()
  const top = box.top + HEADER_HEIGHT
  const bottom = box.top + grid.clientHeight
  return Array.from(grid.querySelectorAll('[role="row"]'))
    .slice(1)
    .filter((row) => {
      const rect = row.getBoundingClientRect()
      const cell = row.querySelector('[role="gridcell"]')
      return (
        rect.height > 0 &&
        rect.top >= top - 1 &&
        rect.bottom <= bottom + 1 &&
        (cell?.textContent ?? "").trim() !== ""
      )
    }).length
}

/** Editor lines drawn entirely inside the editor's own scroller. */
function readableEditorLines(root: HTMLElement) {
  const scroller = root.querySelector<HTMLElement>(".cm-scroller")
  if (!scroller) return 0
  const box = scroller.getBoundingClientRect()
  return Array.from(root.querySelectorAll(".cm-line")).filter((line) => {
    const rect = line.getBoundingClientRect()
    return (
      rect.height > 0 &&
      rect.top >= box.top - 1 &&
      rect.bottom <= box.top + scroller.clientHeight + 1
    )
  }).length
}

async function pushSplitter(
  canvasElement: HTMLElement,
  key: "{End}" | "{Home}"
) {
  const handle = canvasElement.querySelector<HTMLElement>(
    '[data-slot="resizable-handle"]'
  )
  if (!handle) throw new Error("no splitter")
  handle.focus()
  await userEvent.keyboard(key)
}

/**
 * The splitter pushed all the way, in the smallest window Oxyn opens
 * (960 × 600): the workspace leaves a console 500 px tall there, measured
 * under its bar, tabs and status bar. The results keep `MIN_VISIBLE_LINES`
 * rows drawn and the editor as many lines: with a percentage floor the result
 * panel shrank to its header and footer, and the footer announced 250,000
 * rows over a grid that drew none.
 */
export const SplitterKeepsBothSidesReadable: Story = {
  ...Populated,
  decorators: [
    (Story) => (
      <div className="h-[500px] w-[880px] border">
        <Story />
      </div>
    ),
  ],
  play: async ({ canvas, canvasElement }) => {
    const grid = await canvas.findByRole("grid")
    await waitFor(() => expect(readableRows(grid)).toBeGreaterThan(3))

    await pushSplitter(canvasElement, "{End}")
    await waitFor(() =>
      expect(readableRows(grid)).toBeGreaterThanOrEqual(MIN_VISIBLE_LINES)
    )

    await pushSplitter(canvasElement, "{Home}")
    await waitFor(() =>
      expect(readableEditorLines(canvasElement)).toBeGreaterThanOrEqual(
        MIN_VISIBLE_LINES
      )
    )
  },
}

/**
 * The same floor in Comfortable, where a row is 28 px: a floor computed once
 * for Compact would lose a row the moment the density changes.
 */
export const SplitterFloorFollowsDensity: Story = {
  ...SplitterKeepsBothSidesReadable,
  play: async ({ canvas, canvasElement }) => {
    const root = document.documentElement
    const previous = root.dataset.density
    root.dataset.density = "comfortable"
    try {
      const grid = await canvas.findByRole("grid")
      await waitFor(() => expect(readableRows(grid)).toBeGreaterThan(3))
      await pushSplitter(canvasElement, "{End}")
      await waitFor(() =>
        expect(readableRows(grid)).toBeGreaterThanOrEqual(MIN_VISIBLE_LINES)
      )
      await pushSplitter(canvasElement, "{Home}")
      await waitFor(() =>
        expect(readableEditorLines(canvasElement)).toBeGreaterThanOrEqual(
          MIN_VISIBLE_LINES
        )
      )
    } finally {
      if (previous === undefined) delete root.dataset.density
      else root.dataset.density = previous
    }
  },
}

export const CancelledPartial: Story = {
  args: consoleParts({
    notice: "Cancelled by the user after 1,200 rows.",
    result: {
      status: "populated",
      result: "console-view-cancelled",
      columns: invoiceColumns,
      rows: 1_200,
      complete: false,
      truncated: false,
      cancelled: true,
    },
  }),
}

export const RetryableError: Story = {
  args: consoleParts({
    result: {
      status: "error",
      message:
        "could not serialize access due to concurrent update (40001). The statement was not applied.",
      retryable: true,
    },
  }),
}

/** A definitive server error: no retry is offered. */
export const ServerError: Story = {
  args: consoleParts({
    result: {
      status: "error",
      message: 'ERROR: relation "invoices" does not exist (42P01)',
      retryable: false,
    },
  }),
}

/** A value that does not convert is named by its position and type only. */
export const ParameterError: Story = {
  args: consoleParts({
    toolbar: { parameterCount: 1, parametersOpen: true },
    notice: "Parameter $1 is not a valid int64. Nothing was sent.",
    parameters: (
      <ParameterEditor
        rows={[{ id: "p1", type: "int64", text: "twelve" }]}
        onChange={fn()}
        refusal={{
          position: 1,
          expectedType: "int64",
          message: "parameter 1 is not a valid Int64 value",
        }}
      />
    ),
  }),
  play: async ({ canvas }) => {
    await expect(
      canvas.getByRole("region", { name: "Bound parameters" })
    ).toBeVisible()
    // The rejected value appears in its field only, never in a notice.
    await expect(canvas.queryAllByText(/twelve/)).toHaveLength(0)
    await expect(canvas.getByLabelText("Parameter 1 value")).toHaveAttribute(
      "aria-invalid",
      "true"
    )
  },
}

export const Conflict: Story = {
  args: consoleParts({
    toolbar: {
      save: {
        status: "conflict",
        notice:
          "The stored query changed elsewhere. Save a new query to preserve both versions.",
      },
    },
  }),
  play: async ({ canvas }) => {
    await userEvent.click(
      canvas.getByRole("button", { name: "Save as new query" })
    )
    await expect(toolbarDefaults.onSaveAsNew).toHaveBeenCalled()
  },
}

export const ReadOnly: Story = {
  args: consoleParts({
    toolbar: { readOnly: true },
    notice: "This connection is read-only: writes are refused before sending.",
  }),
}

/** The backend is gone: the console keeps its text and says what failed. */
export const BackendUnavailable: Story = {
  args: consoleParts({
    toolbar: {
      save: {
        status: "failed",
        notice: "The query was not saved: the backend is not answering.",
      },
    },
    notice: "The backend is not answering. Your text is kept in this console.",
    result: {
      status: "error",
      message: "The Oxyn backend is not running in this window.",
      retryable: false,
    },
  }),
}

export const HostileText: Story = {
  args: consoleParts({
    toolbar: { title: "‮lqs.seciovni‬ <b>bold</b> مخزن" },
    notice: 'Opened "users"; DROP TABLE audit; -- from history, unrun.',
    text: "SELECT '<script>alert(1)</script>', 'שלום', '🧾' AS \"‮name\";",
  }),
  play: async ({ canvas }) => {
    await expect(canvas.getByText(/DROP TABLE audit/)).toBeVisible()
  },
}

/** The rows of a long statement are readable before it ends. */
function Streaming({
  cancelling = false,
  onCancel,
}: {
  cancelling?: boolean
  onCancel: () => void
}) {
  const [rows, setRows] = React.useState(0)
  React.useEffect(() => {
    if (cancelling) return
    const timer = setInterval(
      () => setRows((count) => Math.min(count + 900, 5_000)),
      200
    )
    return () => clearInterval(timer)
  }, [cancelling])
  const fetchPage = React.useMemo(() => syntheticPages(5_000, 20), [])
  React.useEffect(() => {
    if (cancelling) setRows(1_800)
  }, [cancelling])
  return (
    <ResultPanel
      state={{
        status: "running",
        rows,
        serverCancel: true,
        result: "console-streaming",
        columns: invoiceColumns,
      }}
      fetchPage={fetchPage}
      onCancel={onCancel}
      footerActions={<span className="text-xs">Export…</span>}
      context={{ connectionName: "billing replica", statement: SQL }}
    />
  )
}

/** A long run: the grid fills while the toolbar still says « Running ». */
export const StreamingRows: Story = {
  args: consoleParts({
    toolbar: { running: true, elapsedMs: 8_300 },
    results: <Streaming onCancel={toolbarDefaults.onCancel} />,
  }),
  play: async ({ canvas }) => {
    await waitFor(() => expect(canvas.getByRole("grid")).toBeVisible())
    await waitFor(
      () => expect(canvas.getAllByText("Acme SA").length).toBeGreaterThan(0),
      { timeout: 3000 }
    )
    await expect(canvas.getByRole("button", { name: /^Run$/ })).toBeDisabled()
  },
}

/** Stop pressed while rows arrive: the rows already received stay readable. */
export const CancelledWhileFilling: Story = {
  args: consoleParts({
    toolbar: { running: true, cancelling: true, elapsedMs: 21_000 },
    results: <Streaming cancelling onCancel={toolbarDefaults.onCancel} />,
  }),
  play: async ({ canvas }) => {
    await waitFor(() => expect(canvas.getByRole("grid")).toBeVisible())
    await expect(
      canvas.getByRole("button", { name: /Cancelling/ })
    ).toBeDisabled()
    await expect(
      canvas.getByText("Cancellation requested — waiting for the server")
    ).toBeVisible()
  },
}

/** Find in the rows already loaded: the search never reruns the statement. */
export const FindInResult: Story = {
  args: consoleParts({
    notice: "Returned 5,000 rows in 640 ms.",
    results: (
      <ResultPanel
        state={{
          status: "populated",
          result: "console-find",
          columns: invoiceColumns,
          rows: 5_000,
          complete: true,
          truncated: false,
          cancelled: false,
          elapsedMs: 640,
        }}
        fetchPage={syntheticPages(5_000, 10)}
        toolbar={
          <ResultFindBar
            answer={{
              total: 12,
              row: 42,
              ordinal: 3,
              skippedBatches: 0,
              capped: false,
            }}
            searching={false}
            error={null}
            onFind={toolbarDefaults.onRun}
            onClear={fn()}
          />
        }
        matches={new Set([42, 57])}
        reveal={{ row: 42, key: 1 }}
      />
    ),
  }),
  play: async ({ canvas }) => {
    const field = canvas.getByLabelText("Find in loaded results")
    await userEvent.type(field, "Globex{Enter}")
    await expect(toolbarDefaults.onRun).toHaveBeenCalledWith("Globex", "first")
    await waitFor(() => expect(canvas.getByRole("grid")).toBeVisible())
  },
}

const RELATED_SQL =
  'SELECT *\nFROM "public"."invoices"\nWHERE "customer_id" = $1 AND "issued_on" = $2;'

/**
 * « Related rows » from a selected row: the values are bound in the panel,
 * never written into the statement (I-10), and never echoed in a notice.
 */
export const PrefilledParameters: Story = {
  args: consoleParts({
    toolbar: {
      title: "Related rows.sql",
      parameterCount: 2,
      parametersOpen: true,
    },
    text: RELATED_SQL,
    notice: "Values from the selected row are bound below. Nothing was run.",
    parameters: (
      <ParameterEditor
        rows={[
          { id: "p1", type: "int64", text: "4711" },
          { id: "p2", type: "date", text: "2026-09-02" },
        ]}
        onChange={fn()}
      />
    ),
  }),
  play: async ({ canvas, canvasElement }) => {
    const fields = canvas.getAllByLabelText(/^Parameter \d value$/)
    await expect(fields[0]).toHaveValue("4711")
    await expect(fields[1]).toHaveValue("2026-09-02")
    // The statement keeps its placeholders: no value is written into it.
    const editor = canvasElement.querySelector(".cm-content")
    await expect(editor?.textContent).toContain("$1")
    await expect(editor?.textContent).not.toContain("4711")
    await expect(
      canvas.getByRole("button", { name: /Parameters/ })
    ).toHaveTextContent("2")
  },
}

/** One value could not be read: the console says so before anything runs. */
export const MissingBoundValue: Story = {
  args: consoleParts({
    toolbar: { title: "Related rows.sql", parametersOpen: true },
    text: RELATED_SQL,
    notice: "A value is still missing: fill it in Parameters before running.",
    parameters: (
      <ParameterEditor
        rows={[{ id: "p1", type: "null", text: "" }]}
        onChange={fn()}
      />
    ),
  }),
  play: async ({ canvas }) => {
    await expect(canvas.getByText(/A value is still missing/)).toBeVisible()
  },
}
