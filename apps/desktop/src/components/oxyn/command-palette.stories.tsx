import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, screen, userEvent, waitFor, within } from "storybook/test"

import { CommandPalette } from "./command-palette"
import { NO_FOCUS } from "@/lib/actions/context"
import type {
  ActionSources,
  ConsoleActions,
  WorkspaceActions,
} from "@/lib/actions/context"
import { paletteSections } from "@/lib/actions/listings"
import { manifest } from "@/lib/actions/manifest"

const nothing = () => undefined

const workspace: WorkspaceActions = {
  openConsole: nothing,
  closeActiveTab: nothing,
  nextTab: nothing,
  previousTab: nothing,
  showCatalog: nothing,
  showLibrary: nothing,
  focusPreview: nothing,
  focusConsole: nothing,
  toggleSidebar: nothing,
  toggleAside: nothing,
  openAssistant: nothing,
  switchConnection: nothing,
}

const consoleActions: ConsoleActions = {
  run: nothing,
  runAll: nothing,
  explain: nothing,
  cancel: nothing,
  save: nothing,
}

/** An open workspace on a console, as the registry reads it. */
function sources(running: boolean): ActionSources {
  return {
    workspace: {
      state: {
        activeTab: "console:1",
        tabCount: 1,
        consoleCount: 1,
        objectActive: false,
        hasAside: true,
        hasAssistant: false,
      },
      actions: workspace,
    },
    console: {
      state: { canRun: true, running, cancelling: false, writing: false },
      actions: consoleActions,
    },
    appearance: {
      state: { theme: "dark", density: "compact" },
      actions: { setTheme: nothing, setDensity: nothing },
    },
    overlays: {
      state: {},
      actions: {
        openPalette: nothing,
        openQuickOpen: nothing,
        openShortcuts: nothing,
      },
    },
    settings: { state: {}, actions: { open: nothing } },
  }
}

/** The palette's entries, read from the registry as the window reads them. */
function sections(running: boolean) {
  return paletteSections(manifest, "other", {
    ...NO_FOCUS,
    platform: "other",
    sources: sources(running),
  })
}

const meta = {
  title: "Oxyn/CommandPalette",
  component: CommandPalette,
  args: {
    open: true,
    onOpenChange: fn(),
    sections: sections(false),
    onRun: fn(),
  },
} satisfies Meta<typeof CommandPalette>

export default meta
type Story = StoryObj<typeof meta>

/** Every action by menu, with its combination; a choice shows its mark. */
export const Actions: Story = {
  play: async ({ args }) => {
    const dialog = await screen.findByRole("dialog", {
      name: "Command palette",
    })
    const run = dialog.querySelector('[data-value="console.run"]')
    await expect(run).toHaveTextContent("RunCtrlEnter")
    await expect(
      within(dialog).getByRole("option", { name: /Theme: Dark/ })
    ).toHaveAttribute("data-checked", "true")
    await userEvent.type(
      within(dialog).getByPlaceholderText("Search actions…"),
      "light"
    )
    await userEvent.keyboard("{Enter}")
    await expect(args.onRun).toHaveBeenCalledWith("view.theme.light")
  },
}

/**
 * A greyed action is listed with its reason and reached by the arrows —
 * the reason of a greyed entry of the native macOS bar is read here — and
 * choosing it runs nothing.
 */
export const GreyedWithReason: Story = {
  args: { sections: sections(true) },
  play: async ({ args }) => {
    const dialog = await screen.findByRole("dialog", {
      name: "Command palette",
    })
    await userEvent.type(
      within(dialog).getByPlaceholderText("Search actions…"),
      "run all"
    )
    const option = await within(dialog).findByRole("option", {
      name: /Run all/,
    })
    await expect(option).toHaveAttribute("data-unavailable", "true")
    await expect(option).toHaveAccessibleName(
      "Run all Unavailable: A query is running Ctrl Shift Enter"
    )
    await waitFor(() => expect(option).toHaveAttribute("aria-selected", "true"))
    await userEvent.keyboard("{Enter}")
    await expect(args.onRun).not.toHaveBeenCalled()
  },
}

export const NoMatch: Story = {
  play: async () => {
    const dialog = await screen.findByRole("dialog", {
      name: "Command palette",
    })
    await userEvent.type(
      within(dialog).getByPlaceholderText("Search actions…"),
      "vacuum everything"
    )
    await expect(
      await within(dialog).findByText("No action matches.")
    ).toBeVisible()
  },
}
