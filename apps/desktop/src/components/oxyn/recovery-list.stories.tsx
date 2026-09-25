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
    unresolvedWrite: false,
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
    const [objectSelected, setObjectSelected] = React.useState(
      args.objectSelected ?? false
    )
    return (
      <RecoveryList
        {...args}
        selected={selected}
        objectSelected={objectSelected}
        onToggleObject={() => {
          setObjectSelected((chosen) => !chosen)
          args.onToggleObject?.()
        }}
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
    await expect(
      canvas.getByRole("group", { name: "Working copies to restore" })
    ).toBeInTheDocument()
    const untitled = canvas.getByRole("checkbox", { name: "Untitled query" })
    await expect(untitled).toHaveAccessibleDescription("No connection")
    await userEvent.click(untitled)
    await userEvent.click(
      canvas.getByRole("button", { name: "Restore 1 selected item" })
    )
    await expect(args.onRestore).toHaveBeenCalledTimes(1)
    await expect(
      canvas.getByText(/Nothing executed automatically/)
    ).toBeVisible()
  },
}

/** Clicking the title toggles its box: the label is the checkbox's. */
export const LabelTogglesTheBox: Story = {
  play: async ({ canvas, args }) => {
    await userEvent.click(canvas.getByText("console_1.sql"))
    await expect(args.onToggle).toHaveBeenCalledWith(entries[0])
    await expect(
      canvas.getByRole("checkbox", { name: "console_1.sql" })
    ).not.toBeChecked()
  },
}

/**
 * Opened on demand from the library: no crash to announce, and history holds
 * no write in doubt, so no warning either.
 */
export const OnDemand: Story = {
  args: { abnormal: false, unresolvedWrite: false },
  play: async ({ canvas }) => {
    await expect(canvas.queryByText(/did not close normally/)).toBeNull()
    await expect(
      canvas.getByText(/^Choose the working copies to restore/)
    ).toBeVisible()
    await expect(canvas.queryByText(/unknown outcome/)).toBeNull()
  },
}

/** A write in doubt in history: the screen says to inspect, never retries. */
export const UnresolvedWrite: Story = {
  args: { unresolvedWrite: true },
  play: async ({ canvas }) => {
    await expect(
      canvas.getByText("A write may have an unknown outcome")
    ).toBeVisible()
    await expect(canvas.getByText(/never retries it/)).toBeVisible()
    // Acknowledging happens in History, where the write itself is shown: this
    // screen only points there.
    await expect(canvas.getByText(/mark it reconciled/)).toBeVisible()
    await expect(
      canvas.queryByRole("button", { name: /reconciled/i })
    ).toBeNull()
  },
}

/** Without a selection the action stays in place, disabled. */
export const NothingSelected: Story = {
  args: { selected: new Set(), abnormal: false },
  play: async ({ canvas, args }) => {
    const restore = canvas.getByRole("button", { name: /^Restore/ })
    await expect(restore).toBeDisabled()
    await userEvent.click(restore, { pointerEventsCheck: 0 })
    await expect(args.onRestore).not.toHaveBeenCalled()
  },
}

export const Loading: Story = {
  args: { state: { status: "loading" } },
}

export const NoWorkingCopy: Story = {
  args: { state: { status: "ready", entries: [] }, selected: new Set() },
}

/** A permanent failure: the server's words, and no button to try again. */
export const Failed: Story = {
  args: {
    state: {
      status: "error",
      error: { message: "local state is corrupted", retryable: false },
    },
    selected: new Set(),
  },
  play: async ({ canvas }) => {
    await expect(canvas.getByText("local state is corrupted")).toBeVisible()
    await expect(canvas.getByText(/will fail the same way/)).toBeVisible()
    await expect(canvas.queryByRole("button", { name: "Try again" })).toBeNull()
  },
}

/** A transient failure offers to try again, only on the user's click. */
export const FailedRetryable: Story = {
  args: {
    state: {
      status: "error",
      error: { message: "database is locked", retryable: true },
    },
    selected: new Set(),
  },
  play: async ({ canvas, args }) => {
    await expect(canvas.getByText("database is locked")).toBeVisible()
    await expect(args.onRefresh).not.toHaveBeenCalled()
    await userEvent.click(canvas.getByRole("button", { name: "Try again" }))
    await expect(args.onRefresh).toHaveBeenCalledTimes(1)
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

/**
 * The object tab saved last is offered beside the working copies, chosen by
 * default and counted in the action. It names its sub-view and connection;
 * unticking it leaves it saved.
 */
export const WithAnObjectTab: Story = {
  args: {
    object: {
      name: 'invoices"; DROP TABLE audit; --',
      section: "Relations, incoming",
      connection: "billing replica",
    },
    objectSelected: true,
    onToggleObject: fn(),
  },
  play: async ({ canvas, args }) => {
    const box = canvas.getByRole("checkbox", {
      name: 'invoices"; DROP TABLE audit; --',
    })
    await expect(box).toBeChecked()
    await expect(box).toHaveAccessibleDescription(
      "Relations, incoming · billing replica"
    )
    await expect(
      canvas.getByText(/Nothing is read until you ask/)
    ).toBeVisible()
    await expect(
      canvas.getByRole("button", { name: "Restore 3 selected items" })
    ).toBeEnabled()

    await userEvent.click(box)
    await expect(args.onToggleObject).toHaveBeenCalled()
    await expect(
      canvas.getByRole("button", { name: "Restore 2 selected items" })
    ).toBeEnabled()
    await expect(args.onRestore).not.toHaveBeenCalled()
  },
}

/** Its connection was deleted since: said, not guessed. */
export const ObjectTabOfADeletedConnection: Story = {
  args: {
    state: { status: "ready", entries: [] },
    selected: new Set(),
    object: {
      name: "invoices",
      section: "Data",
      connection: "A deleted connection",
    },
    objectSelected: true,
  },
  play: async ({ canvas }) => {
    await expect(canvas.getByText("Data · A deleted connection")).toBeVisible()
    await expect(
      canvas.getByRole("button", { name: "Restore 1 selected item" })
    ).toBeEnabled()
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
