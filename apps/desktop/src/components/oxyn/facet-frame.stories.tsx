import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent } from "storybook/test"

import { FacetFrame } from "./facet-frame"
import { fetched } from "./metadata-fixtures"

const meta = {
  title: "Oxyn/FacetFrame",
  component: FacetFrame,
  decorators: [
    (Story) => (
      <div className="h-[320px] w-[640px] border">
        <Story />
      </div>
    ),
  ],
  args: {
    label: "constraints",
    freshness: fetched,
    load: { status: "idle" },
    unsupported: null,
    hasValue: true,
    empty: false,
    emptyText: "No constraints reported for this object.",
    onRefresh: fn(),
    onCancel: fn(),
    children: <p className="p-3 text-sm">Two constraints.</p>,
  },
} satisfies Meta<typeof FacetFrame>

export default meta
type Story = StoryObj<typeof meta>

export const Populated: Story = {}

export const NeverLoaded: Story = {
  args: { freshness: { state: "never" }, hasValue: false },
  play: async ({ canvas, args }) => {
    await userEvent.click(
      canvas.getByRole("button", { name: "Load constraints" })
    )
    await expect(args.onRefresh).toHaveBeenCalled()
  },
}

export const Loading: Story = {
  args: {
    freshness: { state: "never" },
    hasValue: false,
    load: { status: "loading" },
  },
  play: async ({ canvas, args }) => {
    // A running read always carries a way to cancel.
    await userEvent.click(canvas.getByRole("button", { name: "Cancel" }))
    await expect(args.onCancel).toHaveBeenCalled()
  },
}

/**
 * Refresh and Cancel keep their own place: the one with nothing to do is
 * dimmed and inert, never replaced by the other under the pointer.
 */
export const RefreshAndCancelNeverSwap: Story = {
  args: { load: { status: "loading" } },
  play: async ({ canvas, args }) => {
    await expect(canvas.getByRole("button", { name: "Refresh" })).toBeDisabled()
    await userEvent.click(canvas.getByRole("button", { name: "Cancel" }))
    await expect(args.onCancel).toHaveBeenCalled()
    await expect(args.onRefresh).not.toHaveBeenCalled()
  },
}

/** Nothing is running: Cancel is there, and inert. */
export const IdleCancelIsInert: Story = {
  play: async ({ canvas }) => {
    await expect(canvas.getByRole("button", { name: "Cancel" })).toBeDisabled()
    await expect(canvas.getByRole("button", { name: "Refresh" })).toBeEnabled()
  },
}

export const Empty: Story = {
  args: { empty: true },
  play: async ({ canvas }) => {
    await expect(
      canvas.getByText("No constraints reported for this object.")
    ).toBeVisible()
    await expect(canvas.queryByRole("alert")).toBeNull()
  },
}

/** A DDL invalidated the cache: the old read stays, marked stale. */
export const Stale: Story = {
  args: { freshness: { state: "invalidated" } },
  play: async ({ canvas }) => {
    await expect(canvas.getByText("Stale")).toBeVisible()
  },
}

export const FailedWithAPreviousRead: Story = {
  args: {
    load: {
      status: "error",
      message:
        "ERROR:  permission denied for table pg_constraint\nSQLSTATE: 42501",
    },
  },
  play: async ({ canvas }) => {
    await expect(canvas.getByText(/42501/)).toBeVisible()
    await expect(canvas.getByText(/it may be outdated/)).toBeVisible()
  },
}

export const Cancelled: Story = {
  args: { load: { status: "cancelled" } },
}

export const NotSupported: Story = {
  args: {
    unsupported: "This session does not support constraint introspection.",
  },
  play: async ({ canvas }) => {
    await expect(canvas.queryByRole("button", { name: "Refresh" })).toBeNull()
  },
}
