import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, screen, userEvent, waitFor } from "storybook/test"

import { UpdateIndicator } from "./update-indicator"

const meta = {
  title: "Oxyn/UpdateIndicator",
  component: UpdateIndicator,
  decorators: [
    (Story) => (
      // Where it sits: the right end of a status bar.
      <div className="flex h-8 items-center justify-end border-t bg-card px-3 text-xs text-muted-foreground">
        <Story />
      </div>
    ),
  ],
  args: {
    pending: { kind: "ready", version: "1.4.0", installOnQuit: true, days: 0 },
    onRestart: fn(),
    onWhatsNew: fn(),
    onOpenSettings: fn(),
    onReleasePage: fn(),
  },
} satisfies Meta<typeof UpdateIndicator>

export default meta
type Story = StoryObj<typeof meta>

/**
 * A downloaded update waits for the quit. `Later` takes the focus — the
 * update can wait — and Esc returns it to the indicator.
 */
export const Ready: Story = {
  play: async ({ canvas, args }) => {
    const trigger = canvas.getByRole("button", {
      name: "Update ready: Oxyn 1.4.0",
    })
    await expect(trigger).toHaveTextContent("Update ready")
    await userEvent.click(trigger)
    const later = await screen.findByRole("button", { name: "Later" })
    await waitFor(() => expect(later).toHaveFocus())
    await waitFor(() =>
      expect(
        screen.getByText(
          "It installs when you quit Oxyn. Oxyn never restarts on its own."
        )
      ).toBeVisible()
    )
    await userEvent.keyboard("{Escape}")
    await waitFor(() => expect(trigger).toHaveFocus())
    await waitFor(() => expect(screen.queryByText(/is ready$/)).toBeNull())
    await expect(args.onRestart).not.toHaveBeenCalled()
  },
}

/** Below 1200 px: the icon alone, still named for assistive technology. */
export const ReadyCompact: Story = {
  args: { compact: true },
  play: async ({ canvas }) => {
    const trigger = canvas.getByRole("button", {
      name: "Update ready: Oxyn 1.4.0",
    })
    await expect(trigger).not.toHaveTextContent("Update ready")
  },
}

/** A week later: a border, and the popover says how long — nothing more. */
export const ReadyStale: Story = {
  args: {
    pending: { kind: "ready", version: "1.4.0", installOnQuit: true, days: 9 },
  },
  play: async ({ canvas }) => {
    const trigger = canvas.getByRole("button", {
      name: "Update ready: Oxyn 1.4.0",
    })
    await expect(trigger).toHaveAttribute("data-stale")
    await userEvent.click(trigger)
    await waitFor(() =>
      expect(screen.getByText("Downloaded 9 days ago.")).toBeVisible()
    )
  },
}

/** Quitting cannot install from here: only `Restart now` does. */
export const ReadyNotInstallableOnQuit: Story = {
  args: {
    pending: {
      kind: "ready",
      version: "1.4.0",
      installOnQuit: false,
      days: 0,
    },
  },
  play: async ({ canvas, args }) => {
    await userEvent.click(canvas.getByRole("button"))
    await waitFor(() =>
      expect(screen.getByText(/can't install it when you quit/)).toBeVisible()
    )
    await userEvent.click(
      await screen.findByRole("button", { name: "Restart now" })
    )
    await expect(args.onRestart).toHaveBeenCalledOnce()
  },
}

/** The download did not carry Oxyn's signature: always said, never quiet. */
export const VerificationFailed: Story = {
  args: {
    pending: {
      kind: "failed",
      failure: "signature",
      version: "1.4.0",
      message:
        "the signature of Oxyn_1.4.0_aarch64.app.tar.gz does not match the public key",
    },
  },
  play: async ({ canvas, args }) => {
    const trigger = canvas.getByRole("button", {
      name: "Update failed: Oxyn 1.4.0",
    })
    await userEvent.click(trigger)
    await waitFor(() =>
      expect(screen.getByText("Oxyn 1.4.0 could not be verified")).toBeVisible()
    )
    await userEvent.click(
      await screen.findByRole("button", { name: "Release page" })
    )
    await expect(args.onReleasePage).toHaveBeenCalledOnce()
  },
}

export const InstallFailed: Story = {
  args: {
    pending: {
      kind: "failed",
      failure: "install",
      version: "1.4.0",
      message: "Permission denied (os error 13): /Applications/Oxyn.app",
    },
  },
  play: async ({ canvas, args }) => {
    await userEvent.click(
      canvas.getByRole("button", { name: "Update failed: Oxyn 1.4.0" })
    )
    await waitFor(() =>
      expect(
        screen.getByText("Oxyn 1.4.0 could not be installed")
      ).toBeVisible()
    )
    await userEvent.click(
      await screen.findByRole("button", { name: "Update settings…" })
    )
    await expect(args.onOpenSettings).toHaveBeenCalledOnce()
    // The popover closes on its way to Settings.
    await waitFor(() =>
      expect(screen.queryByText("Oxyn 1.4.0 could not be installed")).toBeNull()
    )
  },
}
