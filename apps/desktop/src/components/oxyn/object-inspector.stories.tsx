import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent } from "storybook/test"

import { HOSTILE, hostileFacets, invoicesFacets } from "./metadata-fixtures"
import { ObjectInspector } from "./object-inspector"

const meta = {
  title: "Oxyn/ObjectInspector",
  component: ObjectInspector,
  decorators: [
    (Story) => (
      <div className="h-[480px] w-[320px] border">
        <Story />
      </div>
    ),
  ],
  args: { facets: invoicesFacets, loading: false, onCopyName: fn() },
} satisfies Meta<typeof ObjectInspector>

export default meta
type Story = StoryObj<typeof meta>

export const NoSelection: Story = { args: { facets: null } }

export const Loading: Story = {
  args: {
    facets: {
      ...invoicesFacets,
      detail: { freshness: { state: "never" }, value: null },
    },
    loading: true,
  },
}

export const NotRead: Story = {
  args: {
    facets: {
      ...invoicesFacets,
      detail: { freshness: { state: "never" }, value: null },
    },
  },
  play: async ({ canvas }) => {
    await expect(canvas.getByText(/has not been read yet/)).toBeVisible()
  },
}

/** Rows are the engine's estimate, never a count. */
export const Populated: Story = {
  play: async ({ canvas }) => {
    await expect(canvas.getByText("~1,284,512 (estimate)")).toBeVisible()
    await expect(canvas.getByText("392.9 MiB")).toBeVisible()
  },
}

/**
 * A relation named `users"; DROP TABLE audit; --` stays text, and what is
 * copied is the backend's quoted name, not a concatenation (I-10).
 */
export const HostileName: Story = {
  args: { facets: hostileFacets },
  play: async ({ canvas, args }) => {
    await expect(canvas.getByText(HOSTILE)).toBeVisible()
    await expect(canvas.getByText("Stale")).toBeVisible()
    await userEvent.click(
      canvas.getByRole("button", { name: "Copy qualified name" })
    )
    await expect(args.onCopyName).toHaveBeenCalledWith(
      '"public"."users""; DROP TABLE audit; --"'
    )
  },
}
