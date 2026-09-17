import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent } from "storybook/test"

import { ResultFooter } from "./result-footer"
import { Button } from "@/components/ui/button"

const meta = {
  title: "Oxyn/ResultFooter",
  component: ResultFooter,
  decorators: [
    (Story) => (
      <div className="w-[900px] border">
        <Story />
      </div>
    ),
  ],
  args: {
    state: {
      status: "done",
      rows: 200,
      elapsedMs: 84,
      truncated: false,
      cancelled: false,
    },
    onCancel: fn(),
  },
} satisfies Meta<typeof ResultFooter>

export default meta
type Story = StoryObj<typeof meta>

export const PreviewPage: Story = {
  args: {
    note: "Total count not requested",
    actions: (
      <Button size="xs" variant="outline">
        Export preview…
      </Button>
    ),
  },
  play: async ({ canvas }) => {
    await expect(canvas.getByRole("status")).toHaveTextContent(
      "200 rows shown · 84 ms"
    )
    await expect(canvas.getByText("Total count not requested")).toBeVisible()
  },
}

/** Running: the count is visible but not announced at every batch. */
export const Running: Story = {
  args: { state: { status: "running", rows: 48_200, serverCancel: true } },
  play: async ({ canvas, args }) => {
    await expect(canvas.getByRole("status")).toHaveTextContent(/^Running$/)
    await userEvent.click(canvas.getByRole("button", { name: "Cancel" }))
    await expect(args.onCancel).toHaveBeenCalled()
  },
}

export const RunningWithoutServerCancel: Story = {
  args: { state: { status: "running", rows: 0, serverCancel: false } },
  play: async ({ canvas }) => {
    await expect(canvas.getByText(/the server may keep running/)).toBeVisible()
  },
}

export const Truncated: Story = {
  args: {
    state: {
      status: "done",
      rows: 2_000_000,
      elapsedMs: 185_000,
      truncated: true,
      cancelled: false,
    },
  },
  play: async ({ canvas }) => {
    await expect(canvas.getByRole("status")).toHaveTextContent(
      "Truncated · 2,000,000 rows shown, not the whole result · 3 min 05 s"
    )
  },
}

export const Cancelled: Story = {
  args: {
    state: {
      status: "done",
      rows: 3_400,
      elapsedMs: 12_400,
      truncated: false,
      cancelled: true,
    },
  },
}

export const Narrow: Story = {
  args: {
    state: {
      status: "done",
      rows: 1_284_512,
      elapsedMs: 9_870,
      truncated: true,
      cancelled: false,
    },
    note: "Total count not requested",
    actions: (
      <Button size="xs" variant="outline">
        Export…
      </Button>
    ),
  },
  decorators: [
    (Story) => (
      <div className="w-[320px] border">
        <Story />
      </div>
    ),
  ],
}

/**
 * The reserve on cancellation is part of the button, at every width: hidden on
 * a narrow window, Cancel would promise what this session cannot do
 * (docs/UX-SPEC.md, « Annulation »).
 */
export const NarrowWithoutServerCancel: Story = {
  args: { state: { status: "running", rows: 12, serverCancel: false } },
  decorators: [
    (Story) => (
      <div className="w-[420px] border">
        <Story />
      </div>
    ),
  ],
  play: async ({ canvas }) => {
    await expect(canvas.getByText(/the server may keep running/)).toBeVisible()
    await expect(canvas.getByRole("button", { name: "Cancel" })).toBeVisible()
  },
}
