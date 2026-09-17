import * as React from "react"
import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent } from "storybook/test"

import { RecoveryList } from "./recovery-list"
import type { DocumentEntry } from "@/lib/ipc/library"

const entries: Array<DocumentEntry> = [
  {
    id: "d1",
    title: "console_1.sql",
    connection: "018f0000-0000-7000-8000-000000000001",
    connectionName: "billing replica",
    updatedAt: "2026-09-15T09:42:10+00:00",
    isSaved: false,
    isOpen: true,
    hasChanges: true,
    fromAgent: false,
  },
  {
    id: "d2",
    title: "",
    connection: null,
    connectionName: null,
    updatedAt: "2026-09-15T09:30:00+00:00",
    isSaved: false,
    isOpen: true,
    hasChanges: true,
    fromAgent: false,
  },
]

const meta = {
  title: "Oxyn/RecoveryList",
  component: RecoveryList,
  args: {
    abnormal: true,
    state: { status: "ready", entries },
    selected: new Set(entries.map((entry) => entry.id)),
    onToggle: fn(),
    hasPrevious: false,
    hasNext: false,
    onPrevious: fn(),
    onNext: fn(),
    onRefresh: fn(),
    onRestore: fn(),
    onContinue: fn(),
    backLabel: "Back to connections",
    onBack: fn(),
  },
  render: function Render(args) {
    const [selected, setSelected] = React.useState(args.selected)
    return (
      <RecoveryList
        {...args}
        selected={selected}
        onToggle={(entry) => {
          setSelected((current) => {
            const next = new Set(current)
            if (!next.delete(entry.id)) next.add(entry.id)
            return next
          })
          args.onToggle(entry)
        }}
      />
    )
  },
} satisfies Meta<typeof RecoveryList>

export default meta
type Story = StoryObj<typeof meta>

/** Everything is selected; restoring hands back text, and runs nothing. */
export const AfterACrash: Story = {
  play: async ({ canvas, args }) => {
    await expect(canvas.getByText(/Oxyn did not close normally/)).toBeVisible()
    await userEvent.click(canvas.getByLabelText("Restore Untitled query"))
    await userEvent.click(
      canvas.getByRole("button", { name: "Restore 1 selected item" })
    )
    await expect(args.onRestore).toHaveBeenCalledTimes(1)
    await expect(
      canvas.getByText(/Nothing executed automatically/)
    ).toBeVisible()
  },
}

/** Without a selection there is nothing to restore, and no action to press. */
export const NothingSelected: Story = {
  args: { selected: new Set(), abnormal: false },
  play: async ({ canvas }) => {
    await expect(canvas.queryByRole("button", { name: /^Restore/ })).toBeNull()
  },
}

export const Loading: Story = {
  args: { state: { status: "loading" } },
}

export const NoWorkingCopy: Story = {
  args: { state: { status: "ready", entries: [] }, selected: new Set() },
}

export const Failed: Story = {
  args: {
    state: { status: "error", message: "local operation cancelled" },
    selected: new Set(),
  },
}

/** A visible way out, even with nothing to restore. */
export const BackToWorkspace: Story = {
  args: {
    abnormal: false,
    state: { status: "ready", entries: [] },
    backLabel: "Back to workspace",
  },
  play: async ({ canvas, args }) => {
    await userEvent.click(
      canvas.getByRole("button", { name: /Back to workspace/ })
    )
    await expect(args.onBack).toHaveBeenCalled()
  },
}

/**
 * At 420 px the two decisions wrap under the pager: without it « Restore » sat
 * 50 px past the window's edge — the one action this screen exists for.
 */
export const Narrow: Story = {
  args: AfterACrash.args,
  decorators: [
    (Story) => (
      <div data-testid="room" className="w-[420px]">
        <Story />
      </div>
    ),
  ],
  play: async ({ canvas }) => {
    const room = canvas.getByTestId("room").getBoundingClientRect()
    const restore = canvas.getByRole("button", { name: /^Restore \d/ })
    await expect(restore.getBoundingClientRect().right).toBeLessThanOrEqual(
      room.right + 1
    )
  },
}

/** Skipping leaves every working copy where it is: nothing restored, nothing deleted. */
export const SkipForNow: Story = {
  play: async ({ canvas, args }) => {
    const skip = canvas.getByRole("button", { name: "Skip for now" })
    await expect(skip).toHaveAccessibleDescription(/deletes nothing/)
    await userEvent.click(skip)
    await expect(args.onContinue).toHaveBeenCalled()
    await expect(args.onRestore).not.toHaveBeenCalled()
    await expect(
      canvas.getByRole("button", { name: /Back to connections/ })
    ).toHaveAttribute("aria-keyshortcuts", "Escape Meta+BracketLeft")
  },
}
