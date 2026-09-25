import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import { Menu01Icon } from "@hugeicons/core-free-icons"

import {
  Menubar,
  MenubarCheckboxItem,
  MenubarContent,
  MenubarGroup,
  MenubarItem,
  MenubarMenu,
  MenubarSeparator,
  MenubarShortcut,
  MenubarSub,
  MenubarSubContent,
  MenubarSubTrigger,
  MenubarTrigger,
} from "@/components/ui/menubar"
import type {
  EntryState,
  MenuEntry,
  MenuItemModel,
  MenuModel,
} from "@/lib/actions/menu-model"
import { keyCaps } from "@/lib/actions/shortcut"
import type { Platform } from "@/lib/actions/shortcut"

/** The menu of the folded bar, below 1200 px. */
export const FOLDED_MENU = "folded"

/**
 * `label` with its mnemonic underlined while Alt is held. Read whole by
 * screen readers: the split label is presentation only, and an accessible
 * name computed from it would read « F ile ».
 */
function Mnemonic({
  label,
  mnemonic,
  shown,
}: {
  label: string
  mnemonic: string | null
  shown: boolean
}) {
  const at =
    mnemonic === null ? -1 : label.toLowerCase().indexOf(mnemonic.toLowerCase())
  if (!shown || at === -1) return <>{label}</>
  return (
    <>
      <span className="sr-only">{label}</span>
      <span aria-hidden>
        {label.slice(0, at)}
        <span className="underline underline-offset-2">
          {label.slice(at, at + 1)}
        </span>
        {label.slice(at + 1)}
      </span>
    </>
  )
}

/** Whether an entry, or one entry of a submenu, is offered here. */
function offered(
  item: MenuItemModel,
  states: Record<string, EntryState>
): boolean {
  if (item.kind === "submenu")
    return item.groups.some((group) =>
      group.some((entry) => offered(entry, states))
    )
  const state = states[item.id]
  return state !== undefined && state.availability !== "absent"
}

function Entries({
  groups,
  states,
  platform,
  mnemonics,
  onInvoke,
}: {
  groups: Array<Array<MenuItemModel>>
  states: Record<string, EntryState>
  platform: Platform
  mnemonics: boolean
  onInvoke: (id: string) => void
}) {
  const shown = groups
    .map((group) => group.filter((item) => offered(item, states)))
    .filter((group) => group.length > 0)
  return shown.map((group, index) => (
    <React.Fragment key={group[0]?.id ?? index}>
      {index > 0 ? <MenubarSeparator /> : null}
      <MenubarGroup>
        {group.map((item) => {
          if (item.kind === "submenu")
            return (
              <MenubarSub key={item.id}>
                <MenubarSubTrigger label={item.mnemonic ?? undefined}>
                  <Mnemonic
                    label={item.title}
                    mnemonic={item.mnemonic}
                    shown={mnemonics}
                  />
                </MenubarSubTrigger>
                <MenubarSubContent>
                  <Entries
                    groups={item.groups}
                    states={states}
                    platform={platform}
                    mnemonics={mnemonics}
                    onInvoke={onInvoke}
                  />
                </MenubarSubContent>
              </MenubarSub>
            )
          const state = states[item.id]
          return state ? (
            <Entry
              key={item.id}
              entry={item}
              state={state}
              platform={platform}
              mnemonics={mnemonics}
              onInvoke={onInvoke}
            />
          ) : null
        })}
      </MenubarGroup>
    </React.Fragment>
  ))
}

function Entry({
  entry,
  state,
  platform,
  mnemonics,
  onInvoke,
}: {
  entry: MenuEntry
  state: EntryState
  platform: Platform
  mnemonics: boolean
  onInvoke: (id: string) => void
}) {
  const label = entry.labels[state.variant] ?? entry.labels[0] ?? entry.id
  const reason =
    typeof state.availability === "object" ? state.availability.reason : null
  const reasonId = React.useId()
  const shortcut =
    state.shortcut && entry.chord
      ? keyCaps(entry.chord, platform).join(platform === "mac" ? "" : "+")
      : null
  const content = (
    <>
      <span className="flex min-w-0 flex-col">
        <span>
          <Mnemonic label={label} mnemonic={entry.mnemonic} shown={mnemonics} />
        </span>
        {/* The reason is shown with the entry, by pointer and by keyboard
            alike (UX-SPEC, « Une action, un libellé, un raccourci »), and
            read as its description rather than as part of its name. */}
        {reason ? (
          <span
            id={reasonId}
            aria-hidden
            className="text-xs text-muted-foreground"
          >
            {reason}
          </span>
        ) : null}
      </span>
      {shortcut ? <MenubarShortcut>{shortcut}</MenubarShortcut> : null}
    </>
  )
  const common = {
    disabled: reason !== null,
    // The typeahead of Base UI then answers to the mnemonic, as Windows does.
    label: entry.mnemonic ?? undefined,
    "aria-describedby": reason ? reasonId : undefined,
  }
  if (entry.check)
    return (
      <MenubarCheckboxItem
        {...common}
        checked={state.checked ?? false}
        // The registry sets the choice; the mark follows its state.
        onCheckedChange={() => onInvoke(entry.id)}
        closeOnClick
      >
        {content}
      </MenubarCheckboxItem>
    )
  return (
    <MenubarItem {...common} onClick={() => onInvoke(entry.id)}>
      {content}
    </MenubarItem>
  )
}

/**
 * The menu bar of Windows and Linux: the first row of the page, under the
 * system title bar (docs/adr/0041-registre-d-actions-menus-et-raccourcis.md,
 * point 5). Every entry is an action of the registry, with its state computed
 * by the caller; an absent action has no entry, a greyed one says why.
 *
 * Below 1200 px it folds into one menu button. Alt shows the mnemonics and
 * Alt+<letter> opens a menu; F10 reaches the bar — both through the
 * registry's keyboard dispatcher, which calls `onOpenMenuChange`.
 */
export function AppMenubar({
  menus,
  states,
  platform = "other",
  compact,
  mnemonics = false,
  openMenu,
  onOpenMenuChange,
  onInvoke,
}: {
  menus: Array<MenuModel>
  states: Record<string, EntryState>
  platform?: Platform
  compact: boolean
  /** Alt is held: mnemonics are underlined. */
  mnemonics?: boolean
  openMenu: string | null
  onOpenMenuChange: (menu: string | null) => void
  onInvoke: (id: string) => void
}) {
  if (compact) {
    return (
      <Menubar
        data-slot="app-menubar"
        aria-label="Application menu"
        className="h-9 shrink-0 rounded-none border-0 border-b px-2"
      >
        <MenubarMenu
          open={openMenu === FOLDED_MENU}
          onOpenChange={(open) => onOpenMenuChange(open ? FOLDED_MENU : null)}
        >
          <MenubarTrigger aria-label="Menu">
            <HugeiconsIcon icon={Menu01Icon} strokeWidth={2} />
          </MenubarTrigger>
          <MenubarContent>
            {menus.map((menu) => (
              <MenubarSub key={menu.id}>
                <MenubarSubTrigger label={menu.mnemonic ?? undefined}>
                  <Mnemonic
                    label={menu.title}
                    mnemonic={menu.mnemonic}
                    shown={mnemonics}
                  />
                </MenubarSubTrigger>
                <MenubarSubContent>
                  <Entries
                    groups={menu.groups}
                    states={states}
                    platform={platform}
                    mnemonics={mnemonics}
                    onInvoke={onInvoke}
                  />
                </MenubarSubContent>
              </MenubarSub>
            ))}
          </MenubarContent>
        </MenubarMenu>
      </Menubar>
    )
  }
  return (
    <Menubar
      data-slot="app-menubar"
      aria-label="Application menu"
      className="h-9 shrink-0 rounded-none border-0 border-b px-2"
    >
      {menus.map((menu) => (
        <MenubarMenu
          key={menu.id}
          open={openMenu === menu.id}
          onOpenChange={(open) => {
            if (open) onOpenMenuChange(menu.id)
            else if (openMenu === menu.id) onOpenMenuChange(null)
          }}
        >
          <MenubarTrigger>
            <Mnemonic
              label={menu.title}
              mnemonic={menu.mnemonic}
              shown={mnemonics}
            />
          </MenubarTrigger>
          <MenubarContent>
            <Entries
              groups={menu.groups}
              states={states}
              platform={platform}
              mnemonics={mnemonics}
              onInvoke={onInvoke}
            />
          </MenubarContent>
        </MenubarMenu>
      ))}
    </Menubar>
  )
}
