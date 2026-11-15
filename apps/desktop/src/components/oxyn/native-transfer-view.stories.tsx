import * as React from "react"
import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"
import { NativeTransferView } from "./native-transfer-view"
import { NativeTransferForm } from "./native-transfer-form"
import { NativeTransferJobs } from "./native-transfer-jobs"
import { NativeTransferReconcile } from "./native-transfer-reconcile"
import { NativeTransferStopReview } from "./native-transfer-stop-review"
import {
  nativeJobFixture,
  nativeReviewFixture,
  nativeToolFixture,
  nativeViewFixture,
} from "./native-transfer-fixtures"

const meta = {
  title: "Oxyn/NativeTransferView",
  component: NativeTransferView,
  parameters: { layout: "fullscreen" },
  decorators: [
    (Story) => (
      <div className="h-[900px] p-3">
        <Story />
      </div>
    ),
  ],
  args: {
    connection: "Example development",
    state: nativeViewFixture(),
    onOperation: fn(),
    onFormat: fn(),
    onTool: fn(),
    onFile: fn(),
    onOptions: fn(),
    onTrust: fn(),
    onPrepare: fn(),
    onStart: fn(),
    onDiscard: fn(),
    onCancel: fn(),
    onReload: fn(),
    onCancelRead: fn(),
    onReconcile: fn(),
  },
} satisfies Meta<typeof NativeTransferView>
export default meta
type Story = StoryObj<typeof meta>
export const Initial: Story = {
  args: { state: nativeViewFixture({ support: null, historyKnown: false }) },
  play: async ({ canvasElement, args }) => {
    const c = within(canvasElement)
    await expect(c.getByText("Choose your transfer")).toBeVisible()
    await expect(args.onStart).not.toHaveBeenCalled()
    await userEvent.click(
      c.getByRole("button", { name: "Refresh transfer status" })
    )
    await expect(args.onReload).toHaveBeenCalled()
  },
}
export const Empty: Story = {
  play: async ({ canvasElement }) => {
    const c = within(canvasElement)
    await expect(c.getByText("No native transfers yet")).toBeVisible()
    await expect(
      c.getByRole("button", { name: "Prepare transfer" })
    ).toBeDisabled()
  },
}
export const Populated: Story = {
  args: { state: nativeViewFixture({ jobs: [nativeJobFixture()] }) },
}
export const InProgress: Story = {
  args: {
    state: nativeViewFixture({
      busy: "starting",
      review: nativeReviewFixture,
      tool: nativeToolFixture,
      file: {
        grant: "00000000-0000-4000-8000-000000000003",
        name: nativeReviewFixture.job.fileName,
      },
      jobs: [
        nativeJobFixture({
          stage: "running",
          outcome: null,
          command: "example-command",
          finishedAt: null,
        }),
      ],
    }),
  },
  play: async ({ canvasElement, args }) => {
    const c = within(canvasElement)
    await expect(c.getByRole("progressbar")).not.toHaveAttribute(
      "aria-valuenow"
    )
    await userEvent.click(
      c.getByRole("button", { name: "Cancel owned operation" })
    )
    await expect(args.onCancel).toHaveBeenCalled()
    await expect(args.onStart).not.toHaveBeenCalled()
  },
}
export const Error: Story = {
  args: {
    state: nativeViewFixture({
      error: {
        message:
          "The selected native client is older than the required security baseline.",
        retryable: false,
      },
      jobs: [
        nativeJobFixture({
          outcome: "failedBeforeStart",
          failure: "tool",
          bytes: 0,
        }),
      ],
    }),
  },
}
export const UnavailableEngine: Story = {
  args: {
    state: nativeViewFixture({
      support: {
        engine: "mongodb",
        export: false,
        restore: false,
        reason:
          "A native transfer adapter is not available for this database product.",
        exportFormats: [],
        restoreFormats: [],
      },
    }),
  },
  play: async ({ canvasElement, args }) => {
    const c = within(canvasElement)
    await expect(c.getByText("Native transfers are unavailable")).toBeVisible()
    await expect(
      c.getByRole("button", { name: "Select export tool…" })
    ).toBeDisabled()
    await expect(args.onTool).not.toHaveBeenCalled()
  },
}
export const Prepared: Story = {
  args: {
    state: nativeViewFixture({
      review: nativeReviewFixture,
      tool: nativeToolFixture,
      file: {
        grant: "00000000-0000-4000-8000-000000000003",
        name: nativeReviewFixture.job.fileName,
      },
    }),
  },
  play: async ({ canvasElement, args }) => {
    const c = within(canvasElement)
    await expect(c.getByText(nativeReviewFixture.destination)).toBeVisible()
    await expect(args.onStart).not.toHaveBeenCalled()
    await userEvent.click(
      c.getByRole("button", { name: "Start database export" })
    )
    await expect(args.onStart).toHaveBeenCalled()
  },
}
export const RestoreTrustIsExplicit: Story = {
  args: {
    state: nativeViewFixture({
      operation: "restore",
      tool: {
        ...nativeToolFixture,
        identity: {
          ...nativeToolFixture.identity,
          role: "restore",
          name: "pg_restore",
        },
      },
      file: {
        grant: "00000000-0000-4000-8000-000000000003",
        name: "trusted.dump",
      },
    }),
  },
  render: (args) => {
    const [trusted, setTrusted] = React.useState(false)
    return (
      <NativeTransferView
        {...args}
        state={{ ...args.state, trustedArchive: trusted }}
        onTrust={(value) => {
          args.onTrust(value)
          setTrusted(value)
        }}
      />
    )
  },
  play: async ({ canvasElement, args }) => {
    const c = within(canvasElement)
    await expect(
      c.getByRole("button", { name: "Prepare transfer" })
    ).toBeDisabled()
    await userEvent.click(
      c.getByRole("checkbox", { name: /I trust this archive/ })
    )
    await expect(args.onTrust).toHaveBeenCalledWith(true)
    await expect(
      c.getByRole("button", { name: "Prepare transfer" })
    ).toBeEnabled()
    await expect(args.onStart).not.toHaveBeenCalled()
  },
}
const unknown = nativeJobFixture({
  operation: "restore",
  outcome: "unknown",
  failure: "cancelled",
  mayHaveChanged: true,
})
export const UncertainRestore: Story = {
  args: { state: nativeViewFixture({ jobs: [unknown] }) },
  play: async ({ canvasElement, args }) => {
    await userEvent.click(
      within(canvasElement).getByRole("button", {
        name: "I inspected this destination…",
      })
    )
    await expect(args.onReconcile).toHaveBeenCalledWith(unknown)
    await expect(args.onStart).not.toHaveBeenCalled()
  },
}
export const ReconcileReview: Story = {
  render: (args) => (
    <NativeTransferReconcile
      job={unknown}
      onInspect={args.onStart}
      onCancel={args.onCancel}
    />
  ),
  play: async ({ args }) => {
    const c = within(document.body)
    await expect(c.getByRole("button", { name: "Cancel" })).toHaveFocus()
    await expect(args.onStart).not.toHaveBeenCalled()
  },
}
export const StopReview: Story = {
  render: (args) => (
    <NativeTransferStopReview
      connection={args.connection}
      restoring
      stopping={false}
      onStop={args.onCancel}
      onKeep={args.onDiscard}
    />
  ),
  play: async ({ args }) => {
    const c = within(document.body)
    await waitFor(() => expect(c.getByRole("alertdialog")).toBeVisible())
    await expect(
      c.getByRole("button", { name: "Keep transfer open" })
    ).toHaveFocus()
    await userEvent.click(
      c.getByRole("button", { name: "Cancel transfer and close" })
    )
    await expect(args.onCancel).toHaveBeenCalled()
    await expect(args.onDiscard).not.toHaveBeenCalled()
    await waitFor(() => expect(c.getByRole("alertdialog")).toBeVisible())
  },
}
export const Form: Story = {
  render: (args) => (
    <NativeTransferForm
      state={args.state}
      onOperation={args.onOperation}
      onFormat={args.onFormat}
      onTool={args.onTool}
      onFile={args.onFile}
      onOptions={args.onOptions}
      onTrust={args.onTrust}
      onPrepare={args.onPrepare}
    />
  ),
}
export const JobList: Story = {
  render: (args) => (
    <NativeTransferJobs
      jobs={[
        nativeJobFixture({ id: "00000000-0000-4000-8000-000000000003" }),
        unknown,
      ]}
      busy={false}
      onReconcile={args.onReconcile}
    />
  ),
}

export const CompactControlsRemainReachable: Story = {
  decorators: [
    (Story) => (
      <div className="h-[600px] w-[580px]">
        <Story />
      </div>
    ),
  ],
  play: async ({ canvasElement }) => {
    const c = within(canvasElement)
    const view = c.getByRole("region", { name: "Database transfers" })
    const prepare = c.getByRole("button", { name: "Prepare transfer" })
    prepare.scrollIntoView({ block: "center" })
    await waitFor(() => {
      const parent = view.getBoundingClientRect()
      const button = prepare.getBoundingClientRect()
      expect(button.top).toBeGreaterThanOrEqual(parent.top)
      expect(button.bottom).toBeLessThanOrEqual(parent.bottom)
    })
    expect(view.scrollWidth).toBeLessThanOrEqual(view.clientWidth)
  },
}

export const PreparedRestore: Story = {
  args: {
    state: nativeViewFixture({
      operation: "restore",
      trustedArchive: true,
      tool: {
        ...nativeToolFixture,
        identity: {
          ...nativeToolFixture.identity,
          role: "restore",
          name: "pg_restore",
        },
      },
      file: {
        grant: "00000000-0000-4000-8000-000000000003",
        name: "trusted.dump",
      },
      review: {
        destination: nativeReviewFixture.destination,
        job: nativeJobFixture({
          operation: "restore",
          stage: "ready",
          outcome: null,
          finishedAt: null,
          bytes: 0,
          fileName: "trusted.dump",
        }),
      },
    }),
  },
  play: async ({ canvasElement, args }) => {
    const c = within(canvasElement)
    await expect(
      c.getByText("Omit source ownership: yes. Omit source privileges: yes.")
    ).toBeVisible()
    await expect(c.getByText(nativeReviewFixture.destination)).toBeVisible()
    await expect(args.onStart).not.toHaveBeenCalled()
    await userEvent.click(
      c.getByRole("button", { name: "Review destination and restore…" })
    )
    await expect(args.onStart).toHaveBeenCalledTimes(1)
  },
}

export const ReadingAvailabilityAndHistory: Story = {
  args: {
    state: nativeViewFixture({
      support: null,
      historyKnown: false,
      reading: true,
    }),
  },
  play: async ({ canvasElement, args }) => {
    const c = within(canvasElement)
    await expect(c.queryByText("No native transfers yet")).toBeNull()
    await userEvent.click(
      c.getByRole("button", { name: "Cancel status refresh" })
    )
    await expect(args.onCancelRead).toHaveBeenCalledTimes(1)
    await expect(args.onStart).not.toHaveBeenCalled()
  },
}
export const FailedHistoryIsNotEmpty: Story = {
  args: {
    state: nativeViewFixture({
      historyKnown: false,
      error: {
        message: "Native job history could not be read",
        retryable: false,
      },
    }),
  },
  play: async ({ canvasElement }) => {
    const c = within(canvasElement)
    await expect(c.getByText("History has not been read")).toBeVisible()
    await expect(c.queryByText("No native transfers yet")).toBeNull()
  },
}
