import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent } from "storybook/test"

import { SemanticRankingSettings } from "./semantic-ranking-settings"
import type { ModelState, SemanticSnapshot } from "@/lib/ipc/semantic"

function snapshot(model: ModelState, enabled = true): SemanticSnapshot {
  return { enabled, model }
}

const meta = {
  title: "Oxyn/SemanticRankingSettings",
  component: SemanticRankingSettings,
  decorators: [
    (Story) => (
      <div className="max-w-xl p-6">
        <Story />
      </div>
    ),
  ],
  args: {
    snapshot: snapshot({ type: "absent" }, false),
    onEnable: fn(),
    onDisable: fn(),
  },
} satisfies Meta<typeof SemanticRankingSettings>

export default meta
type Story = StoryObj<typeof meta>

/**
 * The initial state, and the default: off, nothing downloaded. The view says
 * what turning it on costs and that nothing leaves the computer, before the
 * switch is touched.
 */
export const Off: Story = {
  play: async ({ canvas, args }) => {
    const toggle = canvas.getByRole("switch", {
      name: "Rank tables by meaning with a local model",
    })
    await expect(toggle).not.toBeChecked()
    await expect(
      canvas.getByText(/no table name, comment or question is sent anywhere/)
    ).toBeVisible()
    await expect(canvas.getByText(/about 220 MB once/)).toBeVisible()
    await expect(canvas.getByText(/about 415 MB on/)).toBeVisible()
    await expect(canvas.getByText(/about 800 MB of memory/)).toBeVisible()
    await expect(canvas.getByRole("status")).toHaveTextContent(
      "Off. Questions are ranked by name only."
    )
    await expect(canvas.queryByRole("button")).toBeNull()
    await userEvent.click(toggle)
    await expect(args.onEnable).toHaveBeenCalledOnce()
    await expect(args.onDisable).not.toHaveBeenCalled()
  },
}

/** A workspace with no data directory: the switch is there, and inert. */
export const Unavailable: Story = {
  args: { snapshot: snapshot({ type: "unavailable" }, false) },
  play: async ({ canvas }) => {
    // Base UI draws the switch as a `span`: disabled is its ARIA state.
    await expect(canvas.getByRole("switch")).toHaveAttribute(
      "aria-disabled",
      "true"
    )
    await expect(canvas.getByRole("status")).toHaveTextContent(/Not available/)
  },
}

export const Verifying: Story = {
  args: { snapshot: snapshot({ type: "verifying" }) },
  play: async ({ canvas, args }) => {
    await expect(canvas.getByRole("status")).toHaveTextContent(
      "Checking the files already on this computer…"
    )
    await expect(
      canvas.getByRole("progressbar", { name: "Model download progress" })
    ).toHaveAttribute("aria-valuetext", "Checking files")
    await userEvent.click(
      canvas.getByRole("button", { name: "Cancel download" })
    )
    await expect(args.onDisable).toHaveBeenCalledOnce()
  },
}

/** The bytes are shown beside the live region, never read aloud. */
export const Downloading: Story = {
  args: {
    snapshot: snapshot({
      type: "downloading",
      received: 88_100_000,
      total: 220_191_240,
    }),
  },
  play: async ({ canvas, args }) => {
    await expect(canvas.getByRole("switch")).toBeChecked()
    const progress = canvas.getByRole("progressbar", {
      name: "Model download progress",
    })
    await expect(progress).toHaveAttribute(
      "aria-valuetext",
      "88.1 MB of 220.2 MB"
    )
    await expect(canvas.getByRole("status")).toHaveTextContent(
      "Downloading the model"
    )
    await expect(canvas.getByRole("status")).not.toHaveTextContent("MB")
    await expect(
      canvas.getByText(/Cancelling turns semantic ranking off/)
    ).toBeVisible()
    await userEvent.click(
      canvas.getByRole("button", { name: "Cancel download" })
    )
    await expect(args.onDisable).toHaveBeenCalledOnce()
  },
}

/** The conversion cannot be cancelled: no button promises it. */
export const Converting: Story = {
  args: { snapshot: snapshot({ type: "converting" }) },
  play: async ({ canvas }) => {
    await expect(canvas.getByRole("status")).toHaveTextContent(
      /cannot be cancelled/
    )
    await expect(
      canvas.queryByRole("button", { name: "Cancel download" })
    ).toBeNull()
    await expect(
      canvas.getByRole("progressbar", { name: "Model download progress" })
    ).toHaveAttribute("aria-valuetext", "Preparing the model")
  },
}

export const Ready: Story = {
  args: { snapshot: snapshot({ type: "ready" }) },
  play: async ({ canvas, args }) => {
    await expect(canvas.getByRole("switch")).toBeChecked()
    await expect(canvas.getByRole("status")).toHaveTextContent(/^Ready\./)
    await expect(
      canvas.getByText("Turning this off deletes the model from this computer.")
    ).toBeVisible()
    await userEvent.click(canvas.getByRole("button", { name: "Delete model" }))
    await expect(args.onDisable).toHaveBeenCalledOnce()
  },
}

/** On, and the model went missing: the panel ranks by name, and says so. */
export const OnWithoutModel: Story = {
  args: { snapshot: snapshot({ type: "absent" }) },
  play: async ({ canvas, args }) => {
    await expect(canvas.getByRole("status")).toHaveTextContent(
      "The model is not downloaded: questions are ranked by name only."
    )
    await userEvent.click(
      canvas.getByRole("button", { name: "Download model" })
    )
    await expect(args.onEnable).toHaveBeenCalledOnce()
  },
}

/** The server's words, whether trying again can help, and the next step. */
export const Failed: Story = {
  args: {
    snapshot: snapshot({
      type: "failed",
      message:
        "Could not download model.safetensors: huggingface.co: connection timed out\ngithub.com: HTTP 503",
      retryable: true,
    }),
  },
  play: async ({ canvas, args }) => {
    await expect(
      canvas.getByText("The local model is not usable")
    ).toBeVisible()
    await expect(canvas.getByText(/connection timed out/)).toBeVisible()
    await userEvent.click(
      canvas.getByRole("button", { name: "Download again" })
    )
    await expect(args.onEnable).toHaveBeenCalledOnce()
    await userEvent.click(canvas.getByRole("button", { name: "Delete model" }))
    await expect(args.onDisable).toHaveBeenCalledOnce()
  },
}

/** A local failure: retrying as is would fail, the next step is said. */
export const FailedLocally: Story = {
  args: {
    snapshot: snapshot({
      type: "failed",
      message: "Could not rename a model file: No space left on device",
      retryable: false,
    }),
  },
  play: async ({ canvas }) => {
    await expect(canvas.getByText(/Check the disk space/)).toBeVisible()
    await expect(
      canvas.getAllByRole("button", { name: "Download again" })
    ).toHaveLength(1)
  },
}

export const Corrupt: Story = {
  args: { snapshot: snapshot({ type: "corrupt" }) },
  play: async ({ canvas, args }) => {
    await expect(canvas.getByText("The model files are damaged")).toBeVisible()
    await userEvent.click(
      canvas.getByRole("button", { name: "Download again" })
    )
    await expect(args.onEnable).toHaveBeenCalledOnce()
  },
}

/** A request on its way: the controls wait instead of queueing a second. */
export const Pending: Story = {
  args: { snapshot: snapshot({ type: "ready" }), pending: true },
  play: async ({ canvas }) => {
    await expect(canvas.getByRole("switch")).toHaveAttribute(
      "aria-disabled",
      "true"
    )
    await expect(
      canvas.getByRole("button", { name: "Delete model" })
    ).toBeDisabled()
  },
}

/** The preference could not be saved: nothing changed, and it says why. */
export const Refused: Story = {
  args: {
    snapshot: snapshot({ type: "absent" }, false),
    refused:
      "Preferences changed elsewhere. Save again to apply your current choices.",
  },
  play: async ({ canvas }) => {
    await expect(canvas.getByText(/Nothing was changed:/)).toBeVisible()
    await expect(canvas.getByRole("switch")).not.toBeChecked()
  },
}
