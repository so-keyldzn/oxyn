// The context menu of each surface, as UX-SPEC lists it (« Menus
// contextuels »): which actions of the registry, in which order and groups
// (docs/adr/0041-registre-d-actions-menus-et-raccourcis.md, point 6).
//
// Nothing here is a label or a condition: an entry is an action's id, and
// its label, shortcut and greying are the action's — the same in the menu
// bar, the palette and on its button. Only a submenu's title lives here,
// since it names a group, not an action.

import type { Zone } from "./manifest"

export type ContextMenuItem =
  string | { submenu: string; items: ReadonlyArray<string> }

export interface SurfaceMenu {
  /** The zone whose label variants the entries take (`Close tab` → `Close`). */
  zone: Zone | null
  /** Entries by group; a separator is drawn between non-empty groups. */
  groups: ReadonlyArray<ReadonlyArray<ContextMenuItem>>
}

const SORT = {
  submenu: "Sort",
  items: ["grid.sortAscending", "grid.sortDescending"],
} as const

export const CONTEXT_MENUS = {
  catalog: {
    zone: "tree",
    groups: [
      [
        "catalog.openData",
        "catalog.viewStructure",
        "catalog.viewDdl",
        "catalog.newConsoleOnSchema",
      ],
      [
        "catalog.copyQualifiedName",
        {
          submenu: "Copy as",
          items: [
            "catalog.copyAs.quotedName",
            "catalog.copyAs.selectAll",
            "catalog.copyAs.insertTemplate",
            "catalog.copyAs.ddl",
          ],
        },
      ],
      ["catalog.refresh", "catalog.collapseAll", "catalog.pin"],
      // Last and apart: a slip lands on anything else (ADR-0042).
      ["object.rename", "object.truncate", "object.drop"],
    ],
  },
  /** A column of the Structure tab: what the tree cannot show (plan, lot 4). */
  structureColumn: {
    zone: null,
    groups: [["object.rename"]],
  },
  connection: {
    zone: null,
    groups: [
      [
        "connection.connect",
        "connection.disconnect",
        "connection.newConsole",
        "connection.refreshCatalog",
      ],
      [
        "connection.edit",
        "connection.duplicate",
        "connection.changeEnvironment",
        "connection.copy",
      ],
      ["connection.delete"],
    ],
  },
  tab: {
    zone: "tabs",
    groups: [
      ["tab.close", "tab.closeOthers", "tab.closeRight", "tab.closeAll"],
      ["tab.duplicate", "tab.rename"],
      ["tab.openInNewWindow", "tab.revealInLibrary"],
    ],
  },
  gridCell: {
    zone: "grid",
    groups: [
      [
        "grid.copyValue",
        {
          submenu: "Copy rows as",
          items: [
            "grid.copyRows.tsv",
            "grid.copyRows.csv",
            "grid.copyRows.json",
            "grid.copyRows.markdown",
            "grid.copyRows.insert",
            "grid.copyRows.inList",
          ],
        },
        "grid.inspectValue",
      ],
      ["grid.filterByValue", "grid.excludeValue", "grid.filterNull", SORT],
      ["grid.hideColumn", "grid.openReferencedRow"],
      ["grid.sendToAssistant"],
    ],
  },
  gridHeader: {
    zone: "grid",
    groups: [
      [SORT, "column.filter"],
      ["column.hide", "column.freeze", "column.autosize"],
      ["column.copyName", "column.copyValues"],
    ],
  },
  editor: {
    zone: "editor",
    groups: [
      ["edit.cut", "edit.copy", "edit.paste"],
      ["editor.runSelection", "editor.runStatement", "console.explain"],
      ["editor.format", "editor.toggleComment"],
      ["editor.openObject", "editor.askAssistant"],
    ],
  },
  library: {
    zone: null,
    groups: [
      ["library.openEntry"],
      ["library.rename", "library.duplicate"],
      ["library.reveal", "library.copyPath"],
      ["library.delete"],
    ],
  },
  assistantAnswer: {
    zone: "assistant",
    groups: [
      ["assistant.copyAnswer", "assistant.copyMarkdown"],
      ["assistant.regenerate"],
    ],
  },
  assistantQuestion: {
    zone: "assistant",
    groups: [["assistant.editQuestion"]],
  },
  // Never `Run`: a proposal is not run by being made (I-07). The test of
  // this module holds it.
  assistantCode: {
    zone: "assistant",
    groups: [["assistant.copyCode", "assistant.openInConsole"]],
  },
  assistantMention: {
    zone: "assistant",
    groups: [["assistant.openObject"]],
  },
  erd: {
    zone: null,
    groups: [
      ["erd.openTable", "erd.copyName"],
      ["erd.relayout", "erd.exportImage"],
    ],
  },
} satisfies Record<string, SurfaceMenu>

export type Surface = keyof typeof CONTEXT_MENUS

/** Every action id a surface's menu lists, submenus included. */
export function surfaceActions(surface: Surface): Array<string> {
  const menu: SurfaceMenu = CONTEXT_MENUS[surface]
  return menu.groups.flatMap((group) =>
    group.flatMap((item) => (typeof item === "string" ? [item] : item.items))
  )
}
