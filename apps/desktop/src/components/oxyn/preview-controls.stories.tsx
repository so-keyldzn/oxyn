import * as React from "react"
import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

import { expectContainedInFrame, openFrame } from "./frame-overflow"
import { PreviewControls, SortToBrowsePages } from "./preview-controls"
import { PLAIN_SHAPE } from "@/lib/ipc/metadata"

const meta = {
  title: "Oxyn/PreviewControls",
  component: PreviewControls,
  decorators: [
    (Story) => (
      <div className="w-[900px] border">
        <Story />
      </div>
    ),
  ],
  args: {
    canFilter: true,
    canSort: true,
    columns: ["id", "customer_id", "amount", "issued_at"],
    applied: PLAIN_SHAPE,
    status: "loaded",
    pagination: { type: "needsOrder" },
    onApplyPredicate: fn(),
    onApplySort: fn(),
    onPage: fn(),
    onCancel: fn(),
    onLoadColumns: fn(),
  },
} satisfies Meta<typeof PreviewControls>

export default meta
type Story = StoryObj<typeof meta>

/**
 * A relation that is not ordered by default (a virtual table, ADR-0068): no
 * page until the user sorts, and the notice opens the Sort menu.
 */
export const UnorderedFirstPage: Story = {
  play: async ({ canvas }) => {
    await expect(canvas.getByRole("status")).toHaveTextContent(
      "Rows come in no guaranteed order, and putting this relation in order may mean reading all of it first. Sort the preview to browse it page by page."
    )
    // Pagination is disabled without a total order: the controls do not exist.
    await expect(canvas.queryByRole("button", { name: /Next page/ })).toBeNull()

    const popover = within(document.body)
    canvas.getByRole("button", { name: "Sort…" }).focus()
    await userEvent.keyboard("{Enter}")
    // The popup fades in: visible once its opening animation has run.
    await waitFor(() =>
      expect(popover.getByRole("button", { name: "Add column" })).toBeVisible()
    )
    await userEvent.keyboard("{Escape}")
    await waitFor(() =>
      expect(popover.queryByRole("button", { name: "Add column" })).toBeNull()
    )
    await waitFor(() =>
      expect(document.querySelector("[data-base-ui-focus-guard]")).toBeNull()
    )
  },
}

/** Before the relation is described, nothing is claimed about its pages. */
export const NotDescribedYet: Story = {
  args: { pagination: { type: "unknown" } },
  play: async ({ canvas }) => {
    await expect(canvas.queryByRole("status")).toBeNull()
    await expect(canvas.queryByRole("button", { name: /Next page/ })).toBeNull()
    await expect(canvas.queryByRole("button", { name: "Sort…" })).toBeNull()
  },
}

/** The first read, in the primary key's order: pages at once (ADR-0068). */
export const KeyOrderFirstPage: Story = {
  args: {
    pagination: {
      type: "ready",
      previous: false,
      next: true,
      firstRow: 1,
      // A column name is received text: drawn as text, never as markup.
      orderedBy: ["tenant_id", "<b>id</b>"],
    },
  },
  play: async ({ canvas, args, canvasElement }) => {
    await expect(canvas.getByRole("status")).toHaveTextContent(
      "Rows come in the order of the primary key (tenant_id, <b>id</b>). Sort the preview to choose another order."
    )
    await expect(canvasElement.querySelector("b")).toBeNull()
    await expect(
      canvas.getByRole("button", { name: /Previous page/ })
    ).toBeDisabled()
    await userEvent.click(canvas.getByRole("button", { name: /Next page/ }))
    await expect(args.onPage).toHaveBeenCalledWith(true)
  },
}

/** A later page of the key order, sent with no sort at all. */
export const KeyOrderSecondPage: Story = {
  args: {
    applied: { ...PLAIN_SHAPE, offset: 200 },
    pagination: {
      type: "ready",
      previous: true,
      next: true,
      firstRow: 201,
      orderedBy: ["id"],
    },
  },
  play: async ({ canvas, args }) => {
    await expect(canvas.getByText(/Rows 201–400 of this order/)).toBeVisible()
    await userEvent.click(canvas.getByRole("button", { name: /Previous page/ }))
    await expect(args.onPage).toHaveBeenCalledWith(false)
  },
}

function HeldByTheView(props: React.ComponentProps<typeof PreviewControls>) {
  const [open, setOpen] = React.useState(false)
  return (
    <div className="flex flex-col">
      <PreviewControls {...props} sortOpen={open} onSortOpenChange={setOpen} />
      <div className="flex justify-end border-t px-2 py-1 text-xs">
        <SortToBrowsePages onClick={() => setOpen(true)} />
      </div>
    </div>
  )
}

/** The menu held by the view: the footer's link opens it, Escape closes it. */
export const SortMenuOpenedFromTheFooter: Story = {
  render: (args) => <HeldByTheView {...args} />,
  play: async ({ canvas }) => {
    const popover = within(document.body)
    canvas.getByRole("button", { name: "Sort to browse pages" }).focus()
    await userEvent.keyboard("{Enter}")
    // The popup fades in: visible once its opening animation has run.
    await waitFor(() =>
      expect(popover.getByRole("button", { name: "Add column" })).toBeVisible()
    )
    await userEvent.keyboard("{Escape}")
    await waitFor(() =>
      expect(popover.queryByRole("button", { name: "Add column" })).toBeNull()
    )
    await waitFor(() =>
      expect(document.querySelector("[data-base-ui-focus-guard]")).toBeNull()
    )
  },
}

export const NoUniqueKey: Story = {
  args: {
    applied: { ...PLAIN_SHAPE, sort: [{ column: "amount", descending: true }] },
    pagination: { type: "noUniqueKey" },
  },
  play: async ({ canvas }) => {
    await expect(canvas.getByText(/no unique key is known/)).toBeVisible()
    await expect(canvas.queryByRole("button", { name: /Next page/ })).toBeNull()
  },
}

export const SecondPageOfATotalOrder: Story = {
  args: {
    applied: {
      sort: [{ column: "amount", descending: true }],
      predicate: null,
      offset: 200,
    },
    pagination: {
      type: "ready",
      previous: true,
      next: true,
      firstRow: 201,
      orderedBy: null,
    },
  },
  play: async ({ canvas, args }) => {
    await expect(canvas.getByText(/Rows 201–400 of this order/)).toBeVisible()
    await userEvent.click(canvas.getByRole("button", { name: /Next page/ }))
    await expect(args.onPage).toHaveBeenCalledWith(true)
  },
}

export const Loading: Story = {
  args: { status: "loading", pagination: null },
}

export const EmptyUnderAFilter: Story = {
  args: {
    status: "empty",
    applied: { ...PLAIN_SHAPE, predicate: "amount > 1000000" },
    pagination: null,
  },
  play: async ({ canvas }) => {
    // Empty is not an error: the read succeeded.
    await expect(canvas.getByText(/No row matches this filter/)).toBeVisible()
  },
}

/** A predicate the driver refused: said in warning, and no page is offered. */
export const RefusedFilter: Story = {
  args: {
    status: "failed",
    applied: { ...PLAIN_SHAPE, predicate: "id >< 3" },
    reshaped: true,
    pagination: {
      type: "ready",
      previous: false,
      next: true,
      firstRow: 1,
      orderedBy: null,
    },
  },
  play: async ({ canvas }) => {
    await expect(canvas.getByRole("status")).toHaveTextContent(
      /This read was refused/
    )
    await expect(canvas.queryByRole("button", { name: /Next page/ })).toBeNull()
  },
}

/**
 * The first read of the table failed: there was no previous shape, so nothing
 * claims one was dropped — the failure below the bar says it all.
 */
export const FirstReadFailed: Story = {
  args: {
    status: "failed",
    applied: PLAIN_SHAPE,
    reshaped: false,
    pagination: null,
  },
  play: async ({ canvas }) => {
    await expect(canvas.queryByRole("status")).toBeNull()
    await expect(canvas.queryByText(/previous shape/)).toBeNull()
  },
}

export const TypingDoesNotRead: Story = {
  play: async ({ canvas, args }) => {
    const field = canvas.getByLabelText("Preview filter predicate")
    await userEvent.type(field, "status = 'active'")
    await expect(args.onApplyPredicate).not.toHaveBeenCalled()
    await expect(canvas.getByText(/not applied yet/)).toBeVisible()
    await userEvent.keyboard("{Enter}")
    await expect(args.onApplyPredicate).toHaveBeenCalledWith(
      "status = 'active'"
    )
  },
}

export const ComposeASortOnTwoColumns: Story = {
  play: async ({ canvas, args }) => {
    await userEvent.click(canvas.getByRole("button", { name: "Sort" }))
    const popover = within(document.body)
    await userEvent.click(
      await popover.findByRole("button", { name: "Add column" })
    )
    await userEvent.click(popover.getByRole("button", { name: "Add column" }))
    await userEvent.selectOptions(
      popover.getByLabelText("Direction of sort column 2"),
      "desc"
    )
    await userEvent.click(popover.getByRole("button", { name: "Apply sort" }))
    await waitFor(() =>
      expect(args.onApplySort).toHaveBeenCalledWith([
        { column: "id", descending: false },
        { column: "customer_id", descending: true },
      ])
    )
    await waitFor(() =>
      expect(popover.queryByRole("button", { name: "Apply sort" })).toBeNull()
    )
    await waitFor(() =>
      expect(document.querySelector("[data-base-ui-focus-guard]")).toBeNull()
    )
  },
}

/**
 * Column names with nothing to break on: the sort keys and their controls
 * stay inside the popover's frame, which stays inside the window.
 */
export const LongColumnNamesStayInTheFrame: Story = {
  args: {
    columns: [
      "shipping_address_line_two_including_building_and_floor_details",
      "aVeryLongCamelCaseColumnNameWithoutAnyUnderscoreToBreakOn",
      ...Array.from(
        { length: 20 },
        (_, index) => `order_line_attribute_${index}`
      ),
    ],
  },
  play: async ({ canvas }) => {
    await userEvent.click(canvas.getByRole("button", { name: "Sort" }))
    const popover = within(document.body)
    await userEvent.click(
      await popover.findByRole("button", { name: "Add column" })
    )
    await userEvent.click(popover.getByRole("button", { name: "Add column" }))
    await expectContainedInFrame(await openFrame("popover-content"))
  },
}

export const ColumnsNotRead: Story = {
  args: { columns: null },
  play: async ({ canvas, args }) => {
    await userEvent.click(canvas.getByRole("button", { name: "Sort" }))
    const popover = within(document.body)
    await userEvent.click(
      await popover.findByRole("button", { name: "Load columns" })
    )
    await expect(args.onLoadColumns).toHaveBeenCalled()
  },
}

/** A session that can neither filter nor sort has no bar at all. */
export const NotSupported: Story = {
  args: { canFilter: false, canSort: false },
  play: async ({ canvasElement }) => {
    await expect(canvasElement.querySelector("input")).toBeNull()
  },
}
