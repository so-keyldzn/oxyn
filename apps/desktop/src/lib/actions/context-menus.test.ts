import { describe, expect, it } from "vitest"

import { menuRows } from "@/components/oxyn/action-menu-items"

import { NO_FOCUS } from "./context"
import type { ActionContext, ActionSources } from "./context"
import { CONTEXT_MENUS, surfaceActions } from "./context-menus"
import type { Surface } from "./context-menus"
import { actionSpec } from "./manifest"
import { behaviours } from "./registry"

const nothing = () => undefined

function contextWith(sources: ActionSources): ActionContext {
  return { ...NO_FOCUS, platform: "mac", sources }
}

function labels(surface: Surface, sources: ActionSources) {
  return menuRows(surface, contextWith(sources)).flatMap((group) =>
    group.flatMap((row) =>
      row.kind === "entry"
        ? [row.entry.label]
        : row.entries.map((entry) => `${row.title} ▸ ${entry.label}`)
    )
  )
}

describe("context menus", () => {
  it("list only actions of the registry", () => {
    for (const surface of Object.keys(CONTEXT_MENUS) as Array<Surface>) {
      for (const id of surfaceActions(surface)) {
        expect(actionSpec(id), `${surface}: ${id}`).toBeDefined()
        expect(behaviours[id], `${surface}: ${id}`).toBeDefined()
      }
    }
  })

  it("show nothing of a surface whose target is not in the context", () => {
    for (const surface of Object.keys(CONTEXT_MENUS) as Array<Surface>) {
      // Their global actions (Copy, Close tab) stand without a target.
      if (surface === "editor" || surface === "tab") continue
      expect(labels(surface, {}), surface).toEqual([])
    }
  })

  it("never offer Run on the assistant's code, whatever the handlers (I-07)", () => {
    const everything = new Proxy(
      {},
      { get: () => nothing, has: () => true }
    ) as Record<string, () => void>
    const sources: ActionSources = {
      assistant: { state: { answering: false }, actions: everything },
      console: {
        state: {
          canRun: true,
          running: false,
          cancelling: false,
          writing: false,
        },
        actions: {
          run: nothing,
          runAll: nothing,
          explain: nothing,
          cancel: nothing,
          save: nothing,
        },
      },
    }
    expect(labels("assistantCode", sources)).toEqual([
      "Copy code",
      "Open in console",
    ])
    expect(
      surfaceActions("assistantCode").filter((id) => /run|explain/i.test(id))
    ).toEqual([])
  })

  it("take the zone's label variant: Close tab reads Close on a tab", () => {
    const sources: ActionSources = {
      workspace: {
        state: {
          activeTab: "console:1",
          tabCount: 2,
          consoleCount: 2,
          objectActive: false,
          hasAside: false,
          hasAssistant: false,
        },
        actions: {
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
        },
      },
      tab: {
        state: { console: true, count: 2, toTheRight: 0, saved: false },
        actions: { close: nothing, closeRight: nothing },
      },
    }
    const rows = menuRows("tab", contextWith(sources)).flat()
    const close = rows.find(
      (row) => row.kind === "entry" && row.entry.id === "tab.close"
    )
    expect(close?.kind === "entry" && close.entry.label).toBe("Close")
    const right = rows.find(
      (row) => row.kind === "entry" && row.entry.id === "tab.closeRight"
    )
    expect(right?.kind === "entry" && right.entry.state).toEqual({
      reason: "No tab is to the right",
    })
  })

  it("grey the preview filters of a console's result: its SQL is never rewritten", () => {
    const sources: ActionSources = {
      grid: {
        state: {
          target: "cell",
          origin: "query",
          filterable: true,
          sortable: true,
          selectedRows: 1,
          oneColumn: true,
          relation: false,
          loadedRows: 10,
          ai: { kind: "none" },
        },
        actions: { sort: nothing, copyRows: nothing, copyValue: nothing },
      },
    }
    const rows = menuRows("gridCell", contextWith(sources)).flat()
    const entries = rows.flatMap((row) =>
      row.kind === "entry" ? [row.entry] : row.entries
    )
    const reason = (id: string) =>
      entries.find((entry) => entry.id === id)?.state
    expect(reason("grid.filterByValue")).toEqual({
      reason: "The SQL you wrote is never rewritten",
    })
    expect(reason("grid.sortAscending")).toEqual({
      reason: "The SQL you wrote is never rewritten",
    })
    expect(reason("grid.copyRows.insert")).toEqual({
      reason: "These rows do not come from one known table",
    })
    // No AI destination: Send to assistant does not exist.
    expect(reason("grid.sendToAssistant")).toBeUndefined()
  })
})
