import * as React from "react"
import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent } from "storybook/test"

import { PLAIN_SHAPE } from "@/lib/ipc/metadata"
import { Button } from "@/components/ui/button"
import { PreviewPagination } from "./preview-pagination"
import { chooseSelectOption } from "./select-story"

const meta = {
  title: "Oxyn/PreviewPagination",
  component: PreviewPagination,
  args: {
    applied: PLAIN_SHAPE,
    pagination: {
      type: "ready",
      previous: false,
      next: false,
      firstRow: 1,
      orderedBy: ["id"],
    },
    rows: 2,
    status: "loaded",
    onPage: fn(),
    onPageSize: fn(),
  },
} satisfies Meta<typeof PreviewPagination>

export default meta
type Story = StoryObj<typeof meta>

export const OnePage: Story = {
  play: async ({ canvas, args }) => {
    await expect(canvas.getByRole("status")).toHaveTextContent(
      "Rows 1–2 · Page 1"
    )
    await expect(
      canvas.getByRole("button", { name: "Previous page" })
    ).toBeDisabled()
    await expect(
      canvas.getByRole("button", { name: "Next page" })
    ).toBeDisabled()
    const size = canvas.getByRole("combobox", { name: "Rows per page" })
    await expect(size).toHaveTextContent("200")
    await chooseSelectOption(size, "50")
    await expect(args.onPageSize).toHaveBeenCalledWith(50)
  },
}

export const FullFirstPage: Story = {
  args: {
    rows: 200,
    pagination: {
      type: "ready",
      previous: false,
      next: true,
      firstRow: 1,
      orderedBy: ["id"],
    },
  },
  play: async ({ canvas, args }) => {
    await expect(canvas.getByRole("status")).toHaveTextContent(
      "Rows 1–200 · Page 1"
    )
    await userEvent.click(canvas.getByRole("button", { name: "Next page" }))
    await expect(args.onPage).toHaveBeenCalledWith(true)
  },
}

export const LastPage: Story = {
  args: {
    applied: { ...PLAIN_SHAPE, pageSize: 50, offset: 50 },
    rows: 1,
    pagination: {
      type: "ready",
      previous: true,
      next: false,
      firstRow: 51,
      orderedBy: ["id"],
    },
  },
  play: async ({ canvas, args }) => {
    await expect(canvas.getByRole("status")).toHaveTextContent(
      "Rows 51–51 · Page 2"
    )
    await expect(
      canvas.getByRole("button", { name: "Next page" })
    ).toBeDisabled()
    const previous = canvas.getByRole("button", { name: "Previous page" })
    previous.focus()
    await userEvent.keyboard("{Enter}")
    await expect(args.onPage).toHaveBeenCalledWith(false)
  },
}

export const Empty: Story = {
  args: { rows: 0, status: "empty" },
  play: async ({ canvas }) => {
    await expect(canvas.getByRole("status")).toHaveTextContent(
      "No rows · Page 1"
    )
    await expect(
      canvas.getByRole("button", { name: "Next page" })
    ).toBeDisabled()
  },
}

export const EmptyAfterFullPage: Story = {
  args: {
    ...LastPage.args,
    rows: 0,
    status: "empty",
  },
  play: async ({ canvas }) => {
    await expect(canvas.getByRole("status")).toHaveTextContent(
      "No rows · Page 2"
    )
    await expect(
      canvas.getByRole("button", { name: "Previous page" })
    ).toBeEnabled()
  },
}

export const Loading: Story = {
  args: { ...FullFirstPage.args, status: "loading", rows: null },
  play: async ({ canvas }) => {
    await expect(
      canvas.getByRole("navigation", { name: "Preview pages" })
    ).toBeVisible()
    await expect(canvas.getByRole("combobox")).toBeDisabled()
    for (const button of canvas.getAllByRole("button"))
      await expect(button).toBeDisabled()
    await expect(canvas.getByRole("status")).toHaveTextContent("Loading rows")
  },
}

export const Initial: Story = {
  args: { status: "initial", rows: null, pagination: null },
}

export const Failed: Story = {
  args: { ...FullFirstPage.args, status: "failed", rows: null },
  play: async ({ canvas }) => {
    await expect(canvas.getByRole("status")).toHaveTextContent(
      "Rows not loaded"
    )
    await expect(
      canvas.getByRole("button", { name: "Next page" })
    ).toBeDisabled()
    await expect(canvas.getByRole("combobox")).toBeEnabled()
  },
}

export const WithoutUniqueKey: Story = {
  args: { pagination: { type: "noUniqueKey" } },
  play: async ({ canvas }) => {
    await expect(
      canvas.getByText(/Only a bounded preview is available/)
    ).toBeVisible()
    await expect(canvas.queryByRole("button", { name: "Next page" })).toBeNull()
    await expect(
      canvas.queryByRole("button", { name: "Previous page" })
    ).toBeNull()
    await expect(canvas.queryByText(/Page 1/)).toBeNull()
    await expect(
      canvas.getByRole("combobox", { name: "Preview rows" })
    ).toBeEnabled()
  },
}
export const NeedsSort: Story = { args: { pagination: { type: "needsOrder" } } }

export const Narrow: Story = {
  decorators: [
    (Story) => (
      <div style={{ width: 360 }}>
        <Story />
      </div>
    ),
  ],
  play: async ({ canvas, canvasElement }) => {
    const bar = canvasElement.querySelector('[data-slot="preview-pagination"]')
    if (!bar) throw new Error("the pager exists")
    const bounds = bar.getBoundingClientRect()
    for (const control of [
      ...canvas.getAllByRole("button"),
      canvas.getByRole("combobox"),
    ]) {
      const rect = control.getBoundingClientRect()
      await expect(rect.right).toBeLessThanOrEqual(bounds.right)
      await expect(rect.bottom).toBeLessThanOrEqual(bounds.bottom)
    }
  },
}

export const RetainedRead: Story = {
  args: {
    applied: { ...PLAIN_SHAPE, offset: 200 },
    rows: 200,
    pagination: { type: "retained", previous: true, next: true, firstRow: 201 },
  },
  play: async ({ canvas, args }) => {
    await expect(
      canvas.getByText("Pages follow one read. Refresh starts a new read.")
    ).toBeVisible()
    await expect(canvas.getByRole("status")).toHaveTextContent(
      "Rows 201–400 · Page 2"
    )
    const previous = canvas.getByRole("button", { name: "Previous page" })
    previous.focus()
    await userEvent.keyboard("{Enter}")
    await expect(args.onPage).toHaveBeenCalledWith(false)
  },
}

export const NativeOrderRequired: Story = {
  args: {
    applied: {
      ...PLAIN_SHAPE,
      sort: [{ column: "status", descending: false }],
    },
    pagination: { type: "needsNativeOrder" },
    rows: 200,
  },
  play: async ({ canvas }) => {
    await expect(
      canvas.getByText("Clear the sort to browse this server's native order.")
    ).toBeVisible()
    await expect(
      canvas.getByRole("button", { name: "Next page" })
    ).toBeDisabled()
  },
}

export const AvailabilityFailed: Story = {
  args: {
    pagination: null,
    error: {
      message: "Catalog metadata is temporarily unavailable",
      retryable: true,
    },
    onRetry: fn(),
  },
  play: async ({ canvas, args }) => {
    await expect(canvas.getByRole("alert")).toHaveTextContent(
      "Page navigation unavailable"
    )
    await expect(canvas.queryByText(/waits for the relation/)).toBeNull()
    await expect(
      canvas.getByRole("button", { name: "Next page" })
    ).toBeDisabled()
    await expect(
      canvas.getByRole("button", { name: "Previous page" })
    ).toBeDisabled()
    await userEvent.click(
      canvas.getByRole("button", { name: "Retry page navigation" })
    )
    await expect(args.onRetry).toHaveBeenCalledOnce()
    await expect(args.onPage).not.toHaveBeenCalled()
  },
}

export const ScalarOrSummary: Story = {
  args: { pagination: { type: "notApplicable" }, rows: 1 },
  play: async ({ canvas }) => {
    await expect(
      canvas.queryByRole("combobox", { name: "Rows per page" })
    ).toBeNull()
    await expect(canvas.queryByRole("button", { name: "Next page" })).toBeNull()
    await expect(
      canvas.queryByRole("button", { name: "Previous page" })
    ).toBeNull()
    await expect(canvas.queryByText(/Page 1/)).toBeNull()
  },
}

/** The same component can gain and lose pagination as its result changes. */
export const PaginationBecomesApplicable: Story = {
  args: {
    pagination: {
      type: "ready",
      previous: false,
      next: true,
      firstRow: 1,
      orderedBy: ["id"],
    },
  },
  render: function Render(args) {
    const [applicable, setApplicable] = React.useState(false)
    return (
      <>
        <Button onClick={() => setApplicable((shown) => !shown)}>
          {applicable ? "Hide page navigation" : "Show page navigation"}
        </Button>
        <PreviewPagination
          {...args}
          pagination={applicable ? args.pagination : { type: "notApplicable" }}
        />
      </>
    )
  },
  play: async ({ canvas, args }) => {
    await expect(
      canvas.queryByRole("combobox", { name: "Rows per page" })
    ).toBeNull()
    await userEvent.click(
      canvas.getByRole("button", { name: "Show page navigation" })
    )
    const size = canvas.getByRole("combobox", { name: "Rows per page" })
    await expect(size).toHaveTextContent("200")
    await expect(canvas.getByRole("status")).toHaveTextContent(
      "Rows 1–2 · Page 1"
    )
    await chooseSelectOption(size, "50")
    await expect(args.onPageSize).toHaveBeenCalledWith(50)
    await userEvent.click(canvas.getByRole("button", { name: "Next page" }))
    await expect(args.onPage).toHaveBeenCalledWith(true)
    await userEvent.click(
      canvas.getByRole("button", { name: "Hide page navigation" })
    )
    await expect(
      canvas.queryByRole("combobox", { name: "Rows per page" })
    ).toBeNull()
    await userEvent.click(
      canvas.getByRole("button", { name: "Show page navigation" })
    )
    await expect(
      canvas.getByRole("combobox", { name: "Rows per page" })
    ).toHaveTextContent("200")
  },
}
