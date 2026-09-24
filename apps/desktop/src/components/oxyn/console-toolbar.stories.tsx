import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, screen, userEvent, waitFor } from "storybook/test"

import { ConsoleToolbar, formatElapsed } from "./console-toolbar"

const meta = {
  title: "Oxyn/ConsoleToolbar",
  component: ConsoleToolbar,
  decorators: [
    (Story) => (
      <div className="w-[1000px]">
        <Story />
      </div>
    ),
  ],
  args: {
    running: false,
    canRun: true,
    readOnly: false,
    target: "statement",
    onRun: fn(),
    onRunAll: fn(),
    onExplain: fn(),
    onCancel: fn(),
    parameterCount: 0,
    parametersOpen: false,
    onToggleParameters: fn(),
    title: "console_1.sql",
    onTitleChange: fn(),
    titleError: null,
    save: { status: "idle", notice: "No saved copy yet." },
    onSave: fn(),
    onSaveAsNew: fn(),
  },
} satisfies Meta<typeof ConsoleToolbar>

export default meta
type Story = StoryObj<typeof meta>

/** Run, Stop and Explain are three buttons at fixed places. */
export const Initial: Story = {
  play: async ({ canvas, args }) => {
    await expect(
      canvas.getByText("⌘Enter executes: current statement")
    ).toBeVisible()
    await userEvent.click(canvas.getByRole("button", { name: /^Run$/ }))
    await expect(args.onRun).toHaveBeenCalled()
    await expect(canvas.getByRole("button", { name: /Stop/ })).toBeDisabled()
    await userEvent.click(canvas.getByRole("button", { name: "Explain" }))
    await expect(args.onExplain).toHaveBeenCalled()
  },
}

export const SelectionTarget: Story = {
  args: { target: "selection" },
  play: async ({ canvas }) => {
    await expect(canvas.getByText("⌘Enter executes: selection")).toBeVisible()
  },
}

export const RunAllFromMenu: Story = {
  play: async ({ canvas, args }) => {
    await userEvent.click(
      canvas.getByRole("button", { name: "More run options" })
    )
    await userEvent.click(
      await screen.findByRole("menuitem", { name: /Run all/ })
    )
    await expect(args.onRunAll).toHaveBeenCalled()
    await waitFor(() => expect(screen.queryByRole("menu")).toBeNull())
  },
}

/**
 * While running, Run and Explain stay where they are but are inert: a second
 * click on the same place cannot start a second statement.
 */
export const Running: Story = {
  args: { running: true, target: "selection", elapsedMs: 12_400 },
  play: async ({ canvas, args }) => {
    await expect(canvas.getByRole("button", { name: /^Run$/ })).toBeDisabled()
    await expect(canvas.getByRole("button", { name: "Explain" })).toBeDisabled()
    await expect(canvas.getByText(/Running · 12\.4 s/)).toBeVisible()
    await userEvent.click(canvas.getByRole("button", { name: /Stop/ }))
    await expect(args.onCancel).toHaveBeenCalled()
  },
}

/** Stop was pressed: it cannot be sent twice while the server answers. */
export const Cancelling: Story = {
  args: { running: true, cancelling: true, elapsedMs: 31_000 },
  play: async ({ canvas }) => {
    const stop = canvas.getByRole("button", { name: /Cancelling/ })
    await expect(stop).toBeDisabled()
    await expect(
      canvas.getByText("Cancellation requested — waiting for the server")
    ).toBeVisible()
  },
}

/** One save at a time, with the way to cancel it beside it. */
export const Saving: Story = {
  args: {
    save: { status: "saving" },
    parameterCount: 2,
    parametersOpen: true,
    onCancelWrite: fn(),
  },
  play: async ({ canvas, args }) => {
    await expect(
      canvas.getByRole("button", { name: /Save query/ })
    ).toBeDisabled()
    await expect(
      canvas.getByRole("button", { name: /Parameters/ })
    ).toHaveAttribute("aria-pressed", "true")
    await userEvent.click(canvas.getByRole("button", { name: "Cancel save" }))
    await expect(args.onCancelWrite).toHaveBeenCalledOnce()
  },
}

/** Nothing to cancel once idle: the action is not offered. */
export const NoCancelWhenIdle: Story = {
  args: { onCancelWrite: fn() },
  play: async ({ canvas }) => {
    await expect(
      canvas.queryByRole("button", { name: /^Cancel (save|close)$/ })
    ).toBeNull()
  },
}

/**
 * Closing: the name stays readable but typing changes nothing, Enter saves
 * nothing, and the close can be cancelled.
 */
export const Closing: Story = {
  args: {
    closing: true,
    onCancelWrite: fn(),
    save: { status: "idle", notice: "Closing the saved query…" },
  },
  play: async ({ canvas, args }) => {
    const name = canvas.getByRole("textbox", { name: "Query name" })
    await expect(name).toHaveValue("console_1.sql")
    await expect(name).toHaveAttribute("readonly")
    await userEvent.type(name, "x{Enter}")
    await expect(args.onTitleChange).not.toHaveBeenCalled()
    await expect(args.onSave).not.toHaveBeenCalled()
    await expect(
      canvas.getByRole("button", { name: /Save query/ })
    ).toBeDisabled()
    await userEvent.click(canvas.getByRole("button", { name: "Cancel close" }))
    await expect(args.onCancelWrite).toHaveBeenCalledOnce()
  },
}

/** The stored query moved: the only save offered writes a new copy. */
export const Conflict: Story = {
  args: {
    save: {
      status: "conflict",
      notice:
        "The stored query changed elsewhere. Save a new query to preserve both versions.",
    },
  },
  play: async ({ canvas, args }) => {
    await expect(
      canvas.queryByRole("button", { name: /^Save query/ })
    ).toBeNull()
    await userEvent.click(
      canvas.getByRole("button", { name: "Save as new query" })
    )
    await expect(args.onSaveAsNew).toHaveBeenCalled()
    await expect(args.onSave).not.toHaveBeenCalled()
  },
}

export const SaveFailed: Story = {
  args: {
    save: {
      status: "failed",
      notice: "The query was not saved: the workspace is read-only on disk.",
    },
  },
}

/** No `SQL` capability: nothing to run, and READ ONLY said up front. */
export const ReadOnlyWithoutSql: Story = {
  args: {
    readOnly: true,
    canRun: false,
    titleError: "Query names must not exceed 256 UTF-8 bytes.",
  },
  play: async ({ canvas }) => {
    await expect(canvas.getByText("READ ONLY")).toBeVisible()
    await expect(canvas.getByRole("button", { name: /^Run$/ })).toBeDisabled()
    await expect(canvas.getByLabelText("Query name")).toHaveAttribute(
      "aria-invalid",
      "true"
    )
  },
}

/** A right-to-left, hostile name stays text and keeps its direction. */
export const HostileRtlName: Story = {
  args: {
    title: "تقرير الفواتير <script>alert(1)</script> ‮gnp.sql",
  },
  play: async ({ canvas }) => {
    const input = canvas.getByLabelText("Query name")
    await expect(input).toHaveAttribute("dir", "auto")
    await expect(input).toHaveValue(
      "تقرير الفواتير <script>alert(1)</script> ‮gnp.sql"
    )
  },
}

/**
 * A refused name is read with the field, not only beside it: `aria-invalid`
 * alone announces « invalid » and never says why.
 */
export const RefusedName: Story = {
  args: {
    title: "a".repeat(300),
    titleError: "This name is longer than 256 bytes; it was not saved.",
  },
  play: async ({ canvas }) => {
    const input = canvas.getByLabelText("Query name")
    await expect(input).toHaveAttribute("aria-invalid", "true")
    const describedBy = input.getAttribute("aria-describedby")
    await expect(describedBy).toBeTruthy()
    await expect(document.getElementById(describedBy ?? "")).toHaveTextContent(
      "longer than 256 bytes"
    )
  },
}

/** The shortcuts a keyboard user is told about are on the buttons themselves. */
export const ShortcutsAreDeclared: Story = {
  args: { running: true },
  play: async ({ canvas }) => {
    await expect(canvas.getByRole("button", { name: "Run" })).toHaveAttribute(
      "aria-keyshortcuts",
      "Meta+Enter"
    )
    await expect(canvas.getByRole("button", { name: /^Stop/ })).toHaveAttribute(
      "aria-keyshortcuts",
      "Escape"
    )
  },
}

/** At the narrowest window the controls wrap; none is hidden. */
export const Narrow: Story = {
  args: { running: true, elapsedMs: 185_000, parameterCount: 12 },
  decorators: [
    (Story) => (
      <div className="w-[520px]">
        <Story />
      </div>
    ),
  ],
  play: async ({ canvas }) => {
    await expect(
      canvas.getByText(new RegExp(formatElapsed(185_000)))
    ).toBeVisible()
  },
}
