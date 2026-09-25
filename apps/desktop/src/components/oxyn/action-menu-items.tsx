import * as React from "react"

import {
  ContextMenuContent,
  ContextMenuGroup,
  ContextMenuItem,
  ContextMenuLabel,
  ContextMenuSeparator,
  ContextMenuShortcut,
  ContextMenuSub,
  ContextMenuSubContent,
  ContextMenuSubTrigger,
} from "@/components/ui/context-menu"
import { menuContext } from "@/lib/actions/context"
import type { ActionContext, ActionSources } from "@/lib/actions/context"
import { CONTEXT_MENUS } from "@/lib/actions/context-menus"
import type { Surface, SurfaceMenu } from "@/lib/actions/context-menus"
import { actionShortcut, actionSpec, labelOf } from "@/lib/actions/manifest"
import type { Zone } from "@/lib/actions/manifest"
import { availability, invoke } from "@/lib/actions/registry"
import type { Availability } from "@/lib/actions/registry"

interface Entry {
  id: string
  label: string
  shortcut: string
  state: Exclude<Availability, "absent">
  destructive: boolean
}

type Row =
  | { kind: "entry"; entry: Entry }
  | { kind: "submenu"; title: string; entries: Array<Entry> }

/** The entry of `id` in `context`, or null where it is absent. */
function entryOf(
  id: string,
  context: ActionContext,
  zone: Zone | null
): Entry | null {
  const spec = actionSpec(id)
  if (!spec) return null
  const state = availability(id, context)
  if (state === "absent") return null
  const variant = spec.variants.find((candidate) => candidate.zone === zone)
  return {
    id,
    label: variant?.label ?? labelOf(spec, context.platform),
    shortcut: actionShortcut(id, context.platform),
    state,
    destructive: spec.destructive,
  }
}

/** The rows of a surface's menu, groups without an entry left out. */
export function menuRows(
  surface: Surface,
  context: ActionContext
): Array<Array<Row>> {
  const menu: SurfaceMenu = CONTEXT_MENUS[surface]
  return menu.groups
    .map((group) =>
      group.flatMap((item): Array<Row> => {
        if (typeof item === "string") {
          const entry = entryOf(item, context, menu.zone)
          return entry ? [{ kind: "entry", entry }] : []
        }
        const entries = item.items.flatMap((id) => {
          const entry = entryOf(id, context, menu.zone)
          return entry ? [entry] : []
        })
        return entries.length > 0
          ? [{ kind: "submenu", title: item.submenu, entries }]
          : []
      })
    )
    .filter((group) => group.length > 0)
}

function EntryItem({
  entry,
  context,
  detail,
}: {
  entry: Entry
  context: ActionContext
  detail: string | undefined
}) {
  const reason = entry.state === true ? undefined : entry.state.reason
  const caption = reason ?? detail
  return (
    <ContextMenuItem
      disabled={entry.state !== true}
      variant={entry.destructive ? "destructive" : "default"}
      onClick={() => invoke(entry.id, "context-menu", context)}
      className={caption ? "flex-col items-stretch gap-0.5" : undefined}
    >
      <span className="flex w-full items-center gap-2">
        {entry.label}
        {entry.shortcut ? (
          <ContextMenuShortcut>{entry.shortcut}</ContextMenuShortcut>
        ) : null}
      </span>
      {caption ? (
        // Part of the item's text: read with it by the keyboard and screen
        // readers, not only on hover (UX-SPEC, « Une action, un libellé »).
        <span className="max-w-60 text-[length:var(--reading-caption)] text-muted-foreground">
          {caption}
        </span>
      ) : null}
    </ContextMenuItem>
  )
}

/**
 * The popup of a surface's context menu: the registry's actions for this
 * surface, evaluated on the right-clicked target.
 *
 * `anchor` is the element the menu was opened on: it stands for the focus,
 * so an entry whose condition depends on the zone reads the zone the user
 * pointed at. `sources` carries the target (`menuContext`). The context is
 * built when the popup renders — on opening — and every entry is invoked
 * with it; `invoke` checks it again, so a click on an entry that stopped
 * applying runs nothing.
 *
 * `details` adds a caption under an enabled entry — `Copy values` says how
 * many loaded rows it copies. A surface with no entry renders nothing.
 */
export function ActionMenuContent({
  surface,
  sources,
  anchor = null,
  title,
  details,
  className,
}: {
  surface: Surface
  sources: ActionSources
  anchor?: Element | null
  title?: string
  details?: Partial<Record<string, string>>
  className?: string
}) {
  const context = menuContext(anchor, sources)
  const groups = menuRows(surface, context)
  if (groups.length === 0) return null
  return (
    <ContextMenuContent className={className}>
      {title ? (
        <ContextMenuGroup>
          <ContextMenuLabel dir="auto" className="max-w-64 truncate">
            {title}
          </ContextMenuLabel>
        </ContextMenuGroup>
      ) : null}
      {groups.map((group, index) => (
        <React.Fragment key={index}>
          {index > 0 ? <ContextMenuSeparator /> : null}
          <ContextMenuGroup>
            {group.map((row) =>
              row.kind === "entry" ? (
                <EntryItem
                  key={row.entry.id}
                  entry={row.entry}
                  context={context}
                  detail={details?.[row.entry.id]}
                />
              ) : (
                <ContextMenuSub key={row.title}>
                  <ContextMenuSubTrigger>{row.title}</ContextMenuSubTrigger>
                  <ContextMenuSubContent>
                    {row.entries.map((entry) => (
                      <EntryItem
                        key={entry.id}
                        entry={entry}
                        context={context}
                        detail={details?.[entry.id]}
                      />
                    ))}
                  </ContextMenuSubContent>
                </ContextMenuSub>
              )
            )}
          </ContextMenuGroup>
        </React.Fragment>
      ))}
    </ContextMenuContent>
  )
}
