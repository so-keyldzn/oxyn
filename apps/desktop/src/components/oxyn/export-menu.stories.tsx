import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

import { ExportMenuView } from "./export-menu"
import type { ExportFormatChoice } from "@/lib/ipc/results"

const formats: Array<ExportFormatChoice> = [
  { format: "csv", label: "CSV", extension: "csv", supported: true },
  { format: "tsv", label: "TSV", extension: "tsv", supported: true },
  { format: "json", label: "JSON", extension: "json", supported: true },
  { format: "jsonl", label: "JSON lines", extension: "jsonl", supported: true },
  {
    format: "parquet",
    label: "Parquet",
    extension: "parquet",
    supported: false,
  },
  { format: "arrow", label: "Arrow IPC", extension: "arrow", supported: true },
  { format: "sql", label: "SQL inserts", extension: "sql", supported: false },
  { format: "markdown", label: "Markdown", extension: "md", supported: false },
]

const meta = {
  title: "Oxyn/ExportMenu",
  component: ExportMenuView,
  decorators: [
    (Story) => (
      <div className="flex h-[420px] w-[520px] items-start justify-end p-4">
        <Story />
      </div>
    ),
  ],
  args: {
    formats,
    formatsFailed: false,
    exportable: true,
    reason: "",
    state: { status: "idle" },
    onExport: fn(),
    onCancel: fn(),
  },
} satisfies Meta<typeof ExportMenuView>

export default meta
type Story = StoryObj<typeof meta>

const body = within(document.body)

async function closeMenu() {
  await userEvent.keyboard("{Escape}")
  await waitFor(() => expect(body.queryByRole("menu")).toBeNull())
}

/** A format that is not written yet is listed, said unavailable, and inert. */
export const Formats: Story = {
  play: async ({ canvas, args }) => {
    await userEvent.click(canvas.getByRole("button", { name: "Export" }))
    const parquet = await body.findByRole("menuitem", { name: /Parquet/ })
    await expect(parquet).toHaveAttribute("aria-disabled", "true")
    await expect(parquet).toHaveTextContent("unavailable")
    await userEvent.click(body.getByRole("menuitem", { name: /^CSV/ }))
    await expect(args.onExport).toHaveBeenCalledWith(formats[0])
    await waitFor(() => expect(body.queryByRole("menu")).toBeNull())
  },
}

/** The reason is reachable by keyboard: the trigger stays focusable. */
export const NotExportable: Story = {
  args: {
    exportable: false,
    reason:
      "This result was truncated at the row limit: it is not the whole result.",
  },
  play: async ({ canvas }) => {
    const trigger = canvas.getByRole("button", { name: /Export/ })
    await expect(trigger).toHaveAttribute("aria-disabled", "true")
    await expect(trigger).toHaveAccessibleDescription(
      /Unavailable: This result was truncated/
    )
    await userEvent.tab()
    await expect(trigger).toHaveFocus()
    // Focus alone shows the reason, without a pointer.
    await waitFor(
      () =>
        expect(
          document.querySelector('[data-slot="tooltip-content"]')
        ).toHaveTextContent(/truncated at the row limit/),
      { timeout: 2000 }
    )
    // Nothing to open: there is no menu behind an unexportable result.
    await userEvent.keyboard("{Enter}")
    await expect(body.queryByRole("menu")).toBeNull()
  },
}

export const PreviewScope: Story = {
  args: { label: "Export preview…", scope: "Export the 200 rows shown as" },
  play: async ({ canvas }) => {
    await userEvent.click(
      canvas.getByRole("button", { name: "Export preview…" })
    )
    const scope = await body.findByText("Export the 200 rows shown as")
    await waitFor(() => expect(scope).toBeVisible())
    await closeMenu()
  },
}

export const FormatsLoading: Story = {
  args: { formats: null },
  play: async ({ canvas }) => {
    await userEvent.click(canvas.getByRole("button", { name: "Export" }))
    const loading = await body.findByText("Reading formats…")
    await waitFor(() => expect(loading).toBeVisible())
    await closeMenu()
  },
}

export const FormatsUnreadable: Story = {
  args: { formats: null, formatsFailed: true },
  play: async ({ canvas }) => {
    await userEvent.click(canvas.getByRole("button", { name: "Export" }))
    const failed = await body.findByText(
      "The export formats could not be read."
    )
    await waitFor(() => expect(failed).toBeVisible())
    await closeMenu()
  },
}

/** A long export is cancellable, and a second cancel is not sent. */
export const Exporting: Story = {
  args: { state: { status: "exporting", format: "CSV", cancelling: false } },
  play: async ({ canvas, args }) => {
    await expect(canvas.getByText("Exporting CSV…")).toBeVisible()
    await userEvent.click(canvas.getByRole("button", { name: "Cancel export" }))
    await expect(args.onCancel).toHaveBeenCalledOnce()
  },
}

export const Cancelling: Story = {
  args: { state: { status: "exporting", format: "CSV", cancelling: true } },
  play: async ({ canvas }) => {
    await expect(canvas.getByText("Cancelling…")).toBeVisible()
    await expect(
      canvas.getByRole("button", { name: "Cancel export" })
    ).toBeDisabled()
  },
}
