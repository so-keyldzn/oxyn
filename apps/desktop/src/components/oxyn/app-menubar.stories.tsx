import * as React from "react"
import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

import { AppMenubar, FOLDED_MENU } from "./app-menubar"
import { NO_FOCUS } from "@/lib/actions/context"
import type {
  ActionSources,
  ConsoleActions,
  WorkspaceActions,
} from "@/lib/actions/context"
import { manifest } from "@/lib/actions/manifest"
import { entryStates, menuBar } from "@/lib/actions/menu-model"
import type { EntryState } from "@/lib/actions/menu-model"

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

/** An open workspace on a console, idle unless told otherwise. */
function withConsole(running: boolean): ActionSources {
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
    settings: { state: {}, actions: { open: nothing } },
    appearance: {
      state: { theme: "dark", density: "compact" },
      actions: { setTheme: nothing, setDensity: nothing },
    },
  }
}

function states(sources: ActionSources): Record<string, EntryState> {
  return Object.fromEntries(
    entryStates(manifest, "other", {
      ...NO_FOCUS,
      platform: "other",
      sources,
    }).map((entry) => [entry.id, entry])
  )
}

const MENUS = menuBar(manifest, "other")

/**
 * Closes the open menu and waits until every popup is gone — Base UI's focus
 * guards included — before axe runs: an open menu leaves `aria-hidden`
 * guards in the page, a state the user is never left in.
 */
async function closeMenus() {
  await userEvent.keyboard("{Escape}")
  await waitFor(() =>
    expect(document.querySelector("[data-base-ui-focus-guard]")).toBeNull()
  )
}

const meta = {
  title: "Oxyn/AppMenubar",
  component: AppMenubar,
  args: {
    menus: MENUS,
    states: states(withConsole(false)),
    compact: false,
    mnemonics: false,
    openMenu: null,
    onOpenMenuChange: fn(),
    onInvoke: fn(),
  },
  render: function Render(args) {
    // The host owns which menu is open; the story does too.
    const [open, setOpen] = React.useState<string | null>(args.openMenu)
    return (
      <AppMenubar
        {...args}
        openMenu={open}
        onOpenMenuChange={(menu) => {
          setOpen(menu)
          args.onOpenMenuChange(menu)
        }}
      />
    )
  },
} satisfies Meta<typeof AppMenubar>

export default meta
type Story = StoryObj<typeof meta>

/** Choosing an entry invokes its action by id, as its button would. */
export const Wide: Story = {
  play: async ({ canvas, args }) => {
    const body = within(document.body)
    await expect(
      canvas.getByRole("menubar", { name: "Application menu" })
    ).toBeVisible()
    await userEvent.click(canvas.getByRole("menuitem", { name: "File" }))
    const entry = await body.findByRole("menuitem", { name: /New console/ })
    await expect(entry).toHaveTextContent("Ctrl+T")
    await userEvent.click(entry)
    await expect(args.onInvoke).toHaveBeenCalledWith("console.new")
    await waitFor(() =>
      expect(document.querySelector("[data-base-ui-focus-guard]")).toBeNull()
    )
  },
}

/** File ends with Exit, Ctrl+Q: the ordered exit of ⌘Q (ADR-0041, point 5). */
export const FileEndsWithExit: Story = {
  args: { openMenu: "file" },
  play: async () => {
    const body = within(document.body)
    const exit = await body.findByRole("menuitem", { name: /Exit/ })
    await expect(exit).toHaveTextContent("Ctrl+Q")
    await expect(exit).not.toHaveAttribute("aria-disabled", "true")
    await closeMenus()
  },
}

/**
 * Run is greyed in the same cases as its button, and says why — read with
 * the entry, by pointer and by keyboard alike.
 */
export const GreyedWithItsReason: Story = {
  args: { states: states(withConsole(true)), openMenu: "query" },
  play: async () => {
    const body = within(document.body)
    const run = await body.findByRole("menuitem", { name: /^Run Ctrl/ })
    await expect(run).toHaveAttribute("aria-disabled", "true")
    await expect(run).toHaveAccessibleDescription("A query is running")
    // Cancel is shown with Escape, which the console's zone binds.
    const cancel = body.getByRole("menuitem", { name: /Cancel/ })
    await expect(cancel).toHaveTextContent("Esc")
    await expect(cancel).not.toHaveAttribute("aria-disabled", "true")
    await closeMenus()
  },
}

/** Without a declared destination, Assistant has no entry at all. */
export const AbsentEntriesAreNotDrawn: Story = {
  args: { openMenu: "view" },
  play: async () => {
    const body = within(document.body)
    await body.findByRole("menuitem", { name: /Toggle sidebar/ })
    await expect(body.queryByRole("menuitem", { name: /Assistant/ })).toBeNull()
    await closeMenus()
  },
}

/**
 * View ▸ Theme is a submenu of choices: the saved one is checked, and
 * choosing another invokes its action — the same choice as the settings.
 */
export const ThemeIsAChoice: Story = {
  args: { openMenu: "view" },
  play: async ({ args }) => {
    const body = within(document.body)
    await userEvent.hover(await body.findByRole("menuitem", { name: "Theme" }))
    const dark = await body.findByRole("menuitemcheckbox", { name: "Dark" })
    await expect(dark).toHaveAttribute("aria-checked", "true")
    await userEvent.click(body.getByRole("menuitemcheckbox", { name: "Light" }))
    await expect(args.onInvoke).toHaveBeenCalledWith("view.theme.light")
    await waitFor(() =>
      expect(document.querySelector("[data-base-ui-focus-guard]")).toBeNull()
    )
  },
}

/** Alt held: the mnemonics are underlined; the names read the same. */
export const MnemonicsShown: Story = {
  args: { mnemonics: true },
  play: async ({ canvas }) => {
    const file = canvas.getByRole("menuitem", { name: "File" })
    await expect(file.querySelector(".underline")).toHaveTextContent("F")
  },
}

/** Below 1200 px the bar folds into one menu button. */
export const Folded: Story = {
  args: { compact: true, openMenu: FOLDED_MENU },
  play: async ({ canvas }) => {
    const body = within(document.body)
    await expect(canvas.getByRole("menuitem", { name: "Menu" })).toBeVisible()
    await userEvent.hover(await body.findByRole("menuitem", { name: "Query" }))
    await waitFor(() =>
      expect(body.getByRole("menuitem", { name: /Run all/ })).toBeVisible()
    )
    // The trigger closes the folded menu and its open submenu at once.
    await userEvent.click(canvas.getByRole("menuitem", { name: "Menu" }))
    await waitFor(() =>
      expect(document.querySelector("[data-base-ui-focus-guard]")).toBeNull()
    )
  },
}

export const Light: Story = {
  globals: { theme: "light" },
}
