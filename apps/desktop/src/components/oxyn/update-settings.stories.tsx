import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent } from "storybook/test"

import { UpdateSettings } from "./update-settings"
import type { UpdateSnapshot, UpdateState } from "@/lib/ipc/updates"

/** Today, a little earlier: the line reads « today at … ». */
const EARLIER = new Date(Date.now() - 5 * 60 * 1000).toISOString()

function snapshot(
  state: UpdateState,
  extra: Partial<UpdateSnapshot> = {}
): UpdateSnapshot {
  return {
    state,
    currentVersion: "1.3.2",
    automatic: true,
    lastCheckedAt: null,
    ...extra,
  }
}

const READY: UpdateState = {
  type: "ready",
  version: "1.4.0",
  notes: null,
  date: "2026-10-01T09:00:00Z",
  readyAt: EARLIER,
  installOnQuit: true,
}

const meta = {
  title: "Oxyn/UpdateSettings",
  component: UpdateSettings,
  decorators: [
    (Story) => (
      <div className="max-w-xl p-6">
        <Story />
      </div>
    ),
  ],
  args: {
    snapshot: snapshot({ type: "idle" }),
    onCheck: fn(),
    onCancel: fn(),
    onDownload: fn(),
    onRestart: fn(),
    onReleasePage: fn(),
    onWhatsNew: fn(),
    onAutomaticChange: fn(),
    onRetrySave: fn(),
  },
} satisfies Meta<typeof UpdateSettings>

export default meta
type Story = StoryObj<typeof meta>

export const NeverChecked: Story = {
  play: async ({ canvas, args }) => {
    await expect(canvas.getByText("Not checked yet.")).toBeVisible()
    await expect(
      canvas.getByText(
        "Applies to Oxyn on this computer, not only to this workspace."
      )
    ).toBeVisible()
    await expect(
      canvas.getByText(/Nothing about your connections, queries or data\./)
    ).toBeVisible()
    await userEvent.click(canvas.getByRole("button", { name: "Check now" }))
    await expect(args.onCheck).toHaveBeenCalledOnce()
  },
}

/**
 * One check at a time: `Check now` stays reachable so the keyboard keeps its
 * place, but does nothing; `Cancel` stops the request.
 */
export const Checking: Story = {
  args: { snapshot: snapshot({ type: "checking" }) },
  play: async ({ canvas, args }) => {
    const check = canvas.getByRole("button", { name: "Check now" })
    check.focus()
    await expect(check).toHaveFocus()
    await expect(check).toHaveAttribute("aria-disabled", "true")
    await userEvent.click(check)
    await expect(args.onCheck).not.toHaveBeenCalled()
    await userEvent.click(canvas.getByRole("button", { name: "Cancel" }))
    await expect(args.onCancel).toHaveBeenCalledOnce()
  },
}

export const UpToDate: Story = {
  args: {
    snapshot: snapshot(
      { type: "upToDate", checkedAt: EARLIER },
      { lastCheckedAt: EARLIER }
    ),
  },
  play: async ({ canvas }) => {
    await expect(
      canvas.getByText(
        /^Oxyn 1\.3\.2 is the latest version\. Last checked today at/
      )
    ).toBeVisible()
  },
}

/** Automatic updates are off: found, and waiting for the user's click. */
export const AvailableManual: Story = {
  args: {
    snapshot: snapshot(
      { type: "available", version: "1.4.0" },
      { automatic: false }
    ),
  },
  play: async ({ canvas, args }) => {
    await expect(canvas.getByText("Oxyn 1.4.0 is available.")).toBeVisible()
    await userEvent.click(
      canvas.getByRole("button", { name: "Download and install" })
    )
    await expect(args.onDownload).toHaveBeenCalledOnce()
  },
}

export const DownloadingKnownTotal: Story = {
  args: {
    snapshot: snapshot({
      type: "downloading",
      version: "1.4.0",
      received: 12_400_000,
      total: 48_000_000,
    }),
  },
  play: async ({ canvas, args }) => {
    const progress = canvas.getByRole("progressbar", {
      name: "Download progress",
    })
    await expect(progress).toHaveAttribute(
      "aria-valuetext",
      "12.4 MB of 48.0 MB"
    )
    // The live region says the state, never the bytes.
    await expect(canvas.getByRole("status")).toHaveTextContent(
      "Downloading Oxyn 1.4.0"
    )
    await expect(canvas.getByRole("status")).not.toHaveTextContent("MB")
    await userEvent.click(canvas.getByRole("button", { name: "Stop download" }))
    await expect(args.onCancel).toHaveBeenCalledOnce()
  },
}

export const DownloadingUnknownTotal: Story = {
  args: {
    snapshot: snapshot({
      type: "downloading",
      version: "1.4.0",
      received: 3_100_000,
      total: null,
    }),
  },
  play: async ({ canvas }) => {
    await expect(canvas.getByRole("status")).toHaveTextContent(
      "Downloading Oxyn 1.4.0"
    )
    await expect(canvas.getByText("— 3.1 MB downloaded")).toBeVisible()
    await expect(
      canvas.getByRole("progressbar", { name: "Download progress" })
    ).not.toHaveAttribute("aria-valuenow")
  },
}

export const Ready: Story = {
  args: { snapshot: snapshot(READY) },
  play: async ({ canvas, args }) => {
    await expect(
      canvas.getByText("Oxyn 1.4.0 is ready. It installs when you quit.")
    ).toBeVisible()
    await userEvent.click(canvas.getByRole("button", { name: "Restart now" }))
    await expect(args.onRestart).toHaveBeenCalledOnce()
  },
}

/** Release notes are text: markup in them is shown, never rendered. */
export const ReadyWithNotes: Story = {
  args: {
    snapshot: snapshot({
      ...READY,
      notes:
        "Faster catalog loading.\n\n<img src=x onerror=alert(1)> **Fixed** a crash on MySQL 5.7.",
    }),
  },
  play: async ({ canvasElement }) => {
    const notes = canvasElement.querySelector("[data-slot=update-notes]")
    await expect(notes).not.toBeNull()
    await expect(notes?.querySelector("img")).toBeNull()
    await expect(notes?.querySelector("strong")).toBeNull()
    await expect(notes).toHaveTextContent("<img src=x onerror=alert(1)>")
    await expect(notes).toHaveTextContent("**Fixed**")
  },
}

export const ReadyNotInstallableOnQuit: Story = {
  args: { snapshot: snapshot({ ...READY, installOnQuit: false }) },
  play: async ({ canvas }) => {
    await expect(
      canvas.getByText(/can't install it when you quit from this location/)
    ).toBeVisible()
  },
}

export const ErrorOffline: Story = {
  args: {
    snapshot: snapshot({
      type: "error",
      kind: "offline",
      message:
        "error sending request for url (https://github.com/…/latest.json): dns error: failed to lookup address information",
      retryable: true,
      version: null,
    }),
  },
  play: async ({ canvas, args }) => {
    await expect(
      canvas.getByText("Oxyn could not reach the update server")
    ).toBeVisible()
    await expect(
      canvas.getByText("Check your network connection.")
    ).toBeVisible()
    await userEvent.click(canvas.getByRole("button", { name: "Check again" }))
    await expect(args.onCheck).toHaveBeenCalledOnce()
  },
}

export const ErrorServer: Story = {
  args: {
    snapshot: snapshot({
      type: "error",
      kind: "server",
      message: "the server answered 502 Bad Gateway",
      retryable: true,
      version: null,
    }),
  },
}

/** Always explicit: nothing was installed, and the manual way is offered. */
export const ErrorSignature: Story = {
  args: {
    snapshot: snapshot({
      type: "error",
      kind: "signature",
      message:
        "the signature of Oxyn_1.4.0_aarch64.app.tar.gz does not match the public key",
      retryable: false,
      version: "1.4.0",
    }),
  },
  play: async ({ canvas, args }) => {
    await expect(
      canvas.getByText(
        "Oxyn 1.4.0 failed signature verification and was discarded"
      )
    ).toBeVisible()
    await expect(canvas.getByText(/^Nothing was installed\./)).toBeVisible()
    await userEvent.click(canvas.getByRole("button", { name: "Release page" }))
    await expect(args.onReleasePage).toHaveBeenCalledOnce()
    await expect(
      canvas.queryByRole("button", { name: "Download again" })
    ).toBeNull()
  },
}

export const ErrorInstall: Story = {
  args: {
    snapshot: snapshot({
      type: "error",
      kind: "install",
      message: "Permission denied (os error 13): /Applications/Oxyn.app",
      retryable: true,
      version: "1.4.0",
    }),
  },
  play: async ({ canvas, args }) => {
    await expect(canvas.getByText("Oxyn 1.3.2 keeps working.")).toBeVisible()
    await userEvent.click(
      canvas.getByRole("button", { name: "Download again" })
    )
    await expect(args.onDownload).toHaveBeenCalledOnce()
  },
}

/** Off by choice: Oxyn checks only when asked, and `Check now` works. */
export const DisabledUser: Story = {
  args: {
    snapshot: snapshot(
      { type: "disabled", reason: "user" },
      { automatic: false }
    ),
  },
  play: async ({ canvas, args }) => {
    await expect(
      canvas.getByRole("switch", {
        name: "Download and install updates automatically",
      })
    ).not.toBeChecked()
    await userEvent.click(canvas.getByRole("button", { name: "Check now" }))
    await expect(args.onCheck).toHaveBeenCalledOnce()
  },
}

/** `OXYN_UPDATES=off`: the switch is shown, off, and says why. */
export const DisabledAdmin: Story = {
  args: {
    snapshot: snapshot({ type: "disabled", reason: "admin" }),
  },
  play: async ({ canvas, args }) => {
    const toggle = canvas.getByRole("switch", {
      name: "Download and install updates automatically",
    })
    await expect(toggle).toHaveAttribute("aria-disabled", "true")
    await expect(toggle).toHaveAccessibleDescription(
      "Updates are turned off by your administrator."
    )
    await userEvent.click(toggle)
    await expect(args.onAutomaticChange).not.toHaveBeenCalled()
    const check = canvas.getByRole("button", { name: "Check now" })
    await expect(check).toHaveAttribute("aria-disabled", "true")
    await userEvent.click(check)
    await expect(args.onCheck).not.toHaveBeenCalled()
  },
}

export const DisabledPackageManager: Story = {
  args: {
    snapshot: snapshot({ type: "disabled", reason: "packageManager" }),
  },
  play: async ({ canvas }) => {
    await expect(canvas.queryByRole("switch")).toBeNull()
    await expect(canvas.queryByRole("button")).toBeNull()
    await expect(
      canvas.getByText(/^Updates are managed by your package manager\./)
    ).toBeVisible()
  },
}

export const DisabledDev: Story = {
  args: {
    snapshot: snapshot({ type: "disabled", reason: "dev" }),
  },
  play: async ({ canvas }) => {
    await expect(canvas.queryByRole("switch")).toBeNull()
    await expect(
      canvas.getByText("Updates are turned off in development builds.")
    ).toBeVisible()
  },
}

/** The switch was applied but not written: it holds until Oxyn quits. */
export const SaveFailed: Story = {
  args: {
    snapshot: snapshot(
      { type: "disabled", reason: "user" },
      { automatic: false }
    ),
    save: {
      status: "failed",
      message:
        "Permission denied (os error 13): ~/Library/Application Support/dev.oxyn/updates.json",
    },
  },
  play: async ({ canvas, args }) => {
    await expect(
      canvas.getByText(/^Applied until Oxyn quits, but not saved:/)
    ).toBeVisible()
    await userEvent.click(canvas.getByRole("button", { name: "Save again" }))
    await expect(args.onRetrySave).toHaveBeenCalledOnce()
  },
}

export const JustUpdated: Story = {
  args: {
    snapshot: snapshot(
      { type: "upToDate", checkedAt: EARLIER },
      { currentVersion: "1.4.0", lastCheckedAt: EARLIER }
    ),
    justUpdated: { from: "1.3.2", at: EARLIER },
  },
  play: async ({ canvas, args }) => {
    await expect(
      canvas.getByText(/^Updated from 1\.3\.2 on today at/)
    ).toBeVisible()
    await userEvent.click(canvas.getByRole("button", { name: "What's new" }))
    await expect(args.onWhatsNew).toHaveBeenCalledOnce()
  },
}
