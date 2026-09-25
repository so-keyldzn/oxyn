import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

import { expectContainedInFrame, openFrame } from "./frame-overflow"
import { ValuePageDialog } from "./value-page-dialog"

const firstPage = {
  column: "notes",
  dataType: "Utf8",
  isNull: false,
  text: "Long note ".repeat(1638),
  offset: 0,
  nextOffset: 16_380,
  totalBytes: 48_213,
}

const meta = {
  title: "Oxyn/ValuePageDialog",
  component: ValuePageDialog,
  args: {
    open: true,
    connectionName: "billing-prod",
    column: "notes",
    row: 49,
    state: { status: "page", page: firstPage },
    canGoBack: false,
    onPrevious: fn(),
    onNext: fn(),
    onRetry: fn(),
    onCopyPage: fn(),
    onClose: fn(),
  },
} satisfies Meta<typeof ValuePageDialog>

export default meta
type Story = StoryObj<typeof meta>

const dialog = within(document.body)

/** Found, then visible once the opening transition is over. */
async function shown(element: Promise<HTMLElement>) {
  const found = await element
  await waitFor(() => expect(found).toBeVisible())
  return found
}

/** Closing while a page loads cancels it. */
export const Loading: Story = {
  args: { state: { status: "loading" } },
  play: async ({ args }) => {
    await shown(dialog.findByText("Loading existing value…"))
    await userEvent.click(
      dialog.getByRole("button", { name: "Cancel inspection" })
    )
    await expect(args.onClose).toHaveBeenCalled()
  },
}

export const FirstOfSeveralPages: Story = {
  play: async ({ args }) => {
    await shown(dialog.findByText(/Rendered bytes 0–16,380 of 48,213/))
    await expect(dialog.getByText(/No SQL executed/)).toBeVisible()
    await expect(
      dialog.getByRole("button", { name: "Previous" })
    ).toBeDisabled()
    await userEvent.click(dialog.getByRole("button", { name: "Next" }))
    await expect(args.onNext).toHaveBeenCalled()
    await userEvent.click(
      dialog.getByRole("button", { name: /Copy this page/ })
    )
    await expect(args.onCopyPage).toHaveBeenCalledWith(firstPage.text)
  },
}

export const Null: Story = {
  args: {
    state: {
      status: "page",
      page: {
        ...firstPage,
        isNull: true,
        text: "",
        nextOffset: null,
        totalBytes: 0,
      },
    },
  },
  play: async () => {
    await shown(dialog.findByText("∅ NULL"))
    await expect(dialog.queryByRole("button", { name: /Copy/ })).toBeNull()
  },
}

export const EmptyValue: Story = {
  args: {
    state: {
      status: "page",
      page: { ...firstPage, text: "", nextOffset: null, totalBytes: 0 },
    },
  },
  play: async () => {
    await shown(dialog.findByText("Empty value (0 bytes)"))
    await expect(dialog.queryByText("∅ NULL")).toBeNull()
  },
}

/** The text `NULL` is a value: shown and copied as text. */
export const TheTextNull: Story = {
  args: {
    state: {
      status: "page",
      page: { ...firstPage, text: "NULL", nextOffset: null, totalBytes: 4 },
    },
  },
  play: async () => {
    const value = await shown(dialog.findByLabelText("Value of notes"))
    await expect(value).toHaveTextContent("NULL")
    await expect(value).not.toHaveAttribute("data-null")
    await expect(
      dialog.getByRole("button", { name: /Copy value/ })
    ).toBeVisible()
  },
}

/** Nothing brings a vanished value back by running the query again. */
export const Expired: Story = {
  args: { state: { status: "expired" } },
  play: async () => {
    await shown(dialog.findByText(/never runs the query again/))
  },
}

/**
 * Long names and a value and an error with nothing to break on: the header,
 * the value and the footer stay inside the frame, and nothing scrolls sideways.
 */
export const LongContentStaysInTheFrame: Story = {
  args: {
    connectionName: "analytics_warehouse_production_eu_west_3_read_replica",
    column: "shipping_address_line_two_including_building_and_floor_details",
    state: {
      status: "page",
      page: { ...firstPage, text: "x".repeat(4_000) },
    },
  },
  play: async () => {
    await expectContainedInFrame(await openFrame("dialog-content"))
  },
}

/** The same with an error message that has nothing to break on. */
export const LongErrorStaysInTheFrame: Story = {
  args: {
    ...LongContentStaysInTheFrame.args,
    state: {
      status: "error",
      message: `spill: /Users/analyst/Library/Caches/oxyn/${"result_batch_".repeat(12)}.arrow could not be read`,
    },
  },
  play: LongContentStaysInTheFrame.play,
}

export const Failed: Story = {
  args: {
    state: {
      status: "error",
      message: "The result batch could not be read from disk: file truncated.",
    },
  },
  play: async ({ args }) => {
    await userEvent.click(
      await dialog.findByRole("button", { name: "Inspect again" })
    )
    await expect(args.onRetry).toHaveBeenCalled()
    // The error keeps a way out, the footer's and the dialog's own.
    for (const close of dialog.getAllByRole("button", { name: "Close" })) {
      await expect(close).toBeEnabled()
    }
  },
}
