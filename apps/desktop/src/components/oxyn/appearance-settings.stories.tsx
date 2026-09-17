import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent } from "storybook/test"

import {
  AppearanceSettings,
  PreferencesSaveStatus,
} from "./appearance-settings"
import { DEFAULT_PREFERENCES } from "@/features/settings/preferences"

const meta = {
  title: "Oxyn/AppearanceSettings",
  component: AppearanceSettings,
  decorators: [
    (Story) => (
      <div className="max-w-xl p-6">
        <Story />
      </div>
    ),
  ],
  args: { preferences: DEFAULT_PREFERENCES, onChange: fn() },
} satisfies Meta<typeof AppearanceSettings>

export default meta
type Story = StoryObj<typeof meta>

export const Defaults: Story = {
  play: async ({ canvas, args }) => {
    await expect(
      canvas.getByRole("button", { name: "Dark", pressed: true })
    ).toBeVisible()
    await userEvent.click(canvas.getByRole("button", { name: "System" }))
    await expect(args.onChange).toHaveBeenCalledWith({ theme: "system" })
    await userEvent.click(canvas.getByRole("button", { name: /^Comfortable/ }))
    await expect(args.onChange).toHaveBeenCalledWith({
      readingDensity: "comfortable",
    })
  },
}

export const LightComfortable: Story = {
  globals: { theme: "light" },
  args: {
    preferences: {
      ...DEFAULT_PREFERENCES,
      theme: "light",
      readingDensity: "comfortable",
    },
  },
}

export const SaveIdle: StoryObj<typeof PreferencesSaveStatus> = {
  render: (args) => <PreferencesSaveStatus {...args} />,
  args: { save: { status: "idle" }, onRetry: fn() },
}

export const Saving: StoryObj<typeof PreferencesSaveStatus> = {
  render: (args) => <PreferencesSaveStatus {...args} />,
  args: { save: { status: "saving" }, onRetry: fn() },
}

export const Saved: StoryObj<typeof PreferencesSaveStatus> = {
  render: (args) => <PreferencesSaveStatus {...args} />,
  args: { save: { status: "saved" }, onRetry: fn() },
}

export const SaveFailed: StoryObj<typeof PreferencesSaveStatus> = {
  render: (args) => <PreferencesSaveStatus {...args} />,
  args: {
    save: {
      status: "failed",
      message:
        "Preferences changed elsewhere. Save again to apply your current choices.",
    },
    onRetry: fn(),
  },
  play: async ({ canvas, args }) => {
    // Never announced as saved; the retry is explicit.
    await expect(canvas.queryByText(/Preferences saved/)).toBeNull()
    await userEvent.click(canvas.getByRole("button", { name: "Save again" }))
    await expect(args.onRetry).toHaveBeenCalled()
  },
}
