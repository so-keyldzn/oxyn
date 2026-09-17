import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent } from "storybook/test"

import { invoicesDetail } from "./fixtures"
import { HOSTILE, unusualDetail, wideDetail } from "./metadata-fixtures"
import { RelationStructure } from "./relation-structure"

const meta = {
  title: "Oxyn/RelationStructure",
  component: RelationStructure,
  decorators: [
    (Story) => (
      <div className="h-[420px]">
        <Story />
      </div>
    ),
  ],
  args: { detail: invoicesDetail, onRefresh: fn() },
} satisfies Meta<typeof RelationStructure>

export default meta
type Story = StoryObj<typeof meta>

export const Loaded: Story = {
  play: async ({ canvas }) => {
    await expect(canvas.getByLabelText("Primary key")).toBeVisible()
    await userEvent.click(canvas.getByRole("button", { name: "Column" }))
    await expect(canvas.getAllByRole("columnheader")[1]).toHaveAttribute(
      "aria-sort",
      "ascending"
    )
    // NULL and NOT NULL read with the same weight: text, not a badge.
    await expect(canvas.getAllByText("NOT NULL")[0]?.tagName).toBe("SPAN")
  },
}

export const Loading: Story = {
  args: { detail: undefined },
  play: async ({ canvas }) => {
    await expect(
      canvas.getByRole("status", { name: "Loading structure" })
    ).toBeVisible()
  },
}

export const NotReadYet: Story = {
  args: { detail: null },
  play: async ({ canvas, args }) => {
    await userEvent.click(
      canvas.getByRole("button", { name: "Refresh structure" })
    )
    await expect(args.onRefresh).toHaveBeenCalled()
  },
}

export const Refreshing: Story = {
  args: { refreshing: true },
  play: async ({ canvas }) => {
    await expect(canvas.getByRole("status")).toHaveTextContent("Refreshing…")
    await expect(canvas.getByLabelText("Primary key")).toBeVisible()
  },
}

/** A failure that may pass: said, with the server's words and a way to try again. */
export const RetryableError: Story = {
  args: {
    detail: null,
    error: {
      message:
        'could not connect to server: Connection timed out\n\tIs the server running on host "db.internal" and accepting TCP/IP connections on port 5432?',
      retryable: true,
    },
  },
  play: async ({ canvas, args }) => {
    await expect(
      canvas.getByText("The structure could not be read")
    ).toBeVisible()
    await expect(canvas.queryByText(/not read yet/)).toBeNull()
    await expect(canvas.getByText(/refreshing may succeed/)).toBeVisible()
    await userEvent.click(
      canvas.getByRole("button", { name: "Refresh structure" })
    )
    await expect(args.onRefresh).toHaveBeenCalled()
  },
}

export const Error: Story = {
  args: {
    detail: null,
    error: {
      message: "ERROR:  permission denied for table invoices\nSQLSTATE: 42501",
      retryable: false,
    },
  },
  play: async ({ canvas }) => {
    await expect(canvas.getByText(/42501/)).toBeVisible()
    await expect(canvas.getByText(/will fail the same way/)).toBeVisible()
  },
}

/** A refresh failed over an earlier read: the columns stay, marked as older. */
export const ErrorOverAnEarlierRead: Story = {
  args: {
    error: {
      message: "canceling statement due to lock timeout",
      retryable: true,
    },
  },
  play: async ({ canvas }) => {
    await expect(canvas.getByText(/come from an earlier read/)).toBeVisible()
    await expect(canvas.getByLabelText("Primary key")).toBeVisible()
  },
}

/** Refused by the connection policy: not an error, and no retry is offered. */
export const Denied: Story = {
  args: {
    detail: null,
    denied: "Catalog introspection is disabled for billing (production).",
  },
  play: async ({ canvas }) => {
    await expect(
      canvas.getByText("Reading the structure was refused")
    ).toBeVisible()
    await expect(
      canvas.queryByRole("button", { name: "Refresh structure" })
    ).toBeNull()
  },
}

/** 1 200 columns: only the visible rows are rendered. */
export const TwelveHundredColumns: Story = {
  args: { detail: wideDetail },
  play: async ({ canvas }) => {
    await expect(canvas.getByText("1,200 columns")).toBeVisible()
    await expect(canvas.getAllByRole("row").length).toBeLessThan(80)
    await expect(canvas.getByRole("table")).toHaveAttribute(
      "aria-rowcount",
      "1201"
    )
  },
}

/** Hostile, right-to-left names stay text. */
export const UnusualNames: Story = {
  args: { detail: unusualDetail },
  play: async ({ canvas }) => {
    await expect(canvas.getByText(HOSTILE)).toBeVisible()
    await expect(canvas.getByText("اسم_العميل")).toHaveAttribute("dir", "auto")
  },
}

export const Narrow: Story = {
  args: { detail: unusualDetail },
  decorators: [
    (Story) => (
      <div className="h-[420px] w-[420px]">
        <Story />
      </div>
    ),
  ],
}
