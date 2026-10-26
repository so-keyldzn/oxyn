import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, within } from "storybook/test"

import { ProviderList } from "./provider-list"
import { localProvider, remoteProvider } from "./assistant-fixtures"
import { draftModels } from "./model-picker-fixtures"
import { providerRows } from "./provider-list-model"

const rows = providerRows({
  providers: [remoteProvider, localProvider],
  agents: [],
  presets: [],
  presetStates: {},
  models: {
    [remoteProvider.id]: { status: "ready", models: draftModels },
    [localProvider.id]: { status: "loading" },
  },
})

const meta = {
  title: "Oxyn/Settings/ProviderList",
  component: ProviderList,
  args: {
    status: "ready",
    loadError: null,
    rows,
    focusRow: null,
    onOpen: fn(),
  },
  decorators: [
    (Story) => (
      <div className="max-w-2xl p-4">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof ProviderList>

export default meta
type Story = StoryObj<typeof meta>

/** Without an agent preset, there is no subscription to sign in with. */
export const WithoutPresets: Story = {
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await expect(canvas.queryByRole("button", { name: /^Sign in/ })).toBeNull()
    await expect(canvas.getByText("2 of 4 declared")).toBeVisible()
    // The endpoint-dependent family is marked, and the mark is not the text.
    await userEvent.click(
      canvas.getByRole("button", {
        name: "OpenAI-compatible endpoint, Checking status…",
      })
    )
    await expect(args.onOpen).toHaveBeenCalledWith(
      expect.objectContaining({ id: "family:openai_compatible" })
    )
  },
}

/** Coming back from a provider, the focus lands on its row. */
export const FocusReturnsToTheRow: Story = {
  args: { focusRow: "family:anthropic" },
  play: async ({ canvasElement }) => {
    await expect(
      within(canvasElement).getByRole("button", {
        name: "Anthropic API, Connected",
      })
    ).toHaveFocus()
  },
}
