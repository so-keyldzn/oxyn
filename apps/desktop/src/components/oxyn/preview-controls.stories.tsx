import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

import { expectContainedInFrame, openFrame } from "./frame-overflow"
import { PreviewControls } from "./preview-controls"
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

/** The plain first preview: no order, so no page (ADR-0028). */
export const UnorderedFirstPage: Story = {
  play: async ({ canvas }) => {
    await expect(canvas.getByText(/no guaranteed order/)).toBeVisible()
    // Pagination is disabled without a total order: the controls do not exist.
    await expect(canvas.queryByRole("button", { name: /Next page/ })).toBeNull()
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
    pagination: { type: "ready", previous: true, next: true, firstRow: 201 },
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
    pagination: { type: "ready", previous: false, next: true, firstRow: 1 },
  },
  play: async ({ canvas }) => {
    await expect(canvas.getByRole("status")).toHaveTextContent(
      /This read was refused/
    )
    await expect(canvas.queryByRole("button", { name: /Next page/ })).toBeNull()
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
    await userEvent.click(canvas.getByRole("button", { name: /Sort/ }))
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
    await userEvent.click(canvas.getByRole("button", { name: /Sort/ }))
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
    await userEvent.click(canvas.getByRole("button", { name: /Sort/ }))
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
