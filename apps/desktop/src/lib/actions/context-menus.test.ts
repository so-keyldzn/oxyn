import { describe, expect, it } from "vitest"

import { menuRows } from "@/components/oxyn/action-menu-items"

import { NO_FOCUS } from "./context"
import type { ActionContext, ActionSources } from "./context"
import { CONTEXT_MENUS, surfaceActions } from "./context-menus"
import type { Surface } from "./context-menus"
import { actionSpec } from "./manifest"
import { behaviours } from "./registry"
import menuBehavioursSource from "./menu-behaviours.ts?raw"
import registrySource from "./registry.ts?raw"
import objectOperationsSource from "@/components/oxyn/object-operations.ts?raw"

const nothing = () => undefined

/**
 * A greyed entry's reason that names no way out: « Not available for this
 * tab », « not available here ». UX-SPEC wants the reason read; it must also
 * say where the action is done.
 */
const GENERIC =
  /not available (for|in|on) this|not available here|available for this/i

// Where menu reasons are written.
const REASON_SOURCES = {
  "menu-behaviours.ts": menuBehavioursSource,
  "registry.ts": registrySource,
  "object-operations.ts": objectOperationsSource,
}

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
          shownColumns: 3,
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

  it("never lose an entry because its surface left the handler out", () => {
    const everything = new Proxy(
      {},
      { get: () => nothing, has: () => true }
    ) as Record<string, () => void>
    // The same targets, handlers given or not: a surface that forgets one
    // greys the entry with a reason, and two targets of the same kind keep
    // the same menu (UX-SPEC, « Une action, un libellé, un raccourci »).
    const workspace = (actions: Record<string, () => void>) => ({
      state: {
        activeTab: "console:1",
        tabCount: 3,
        consoleCount: 3,
        objectActive: false,
        hasAside: false,
        hasAssistant: true,
      },
      actions: actions as never,
    })
    const offered = { state: "offered" } as const
    const targets = (actions: Record<string, () => void>) =>
      ({
        catalog: {
          catalogNode: {
            state: {
              relation: true,
              holdsRecords: true,
              schemaContext: false,
              definition: true,
              expanded: true,
              pin: true,
            },
            actions,
          },
          objectOperation: {
            state: {
              offers: { rename: offered, truncate: offered, drop: offered },
            },
            actions,
          },
        },
        structureColumn: {
          objectOperation: {
            state: {
              offers: {
                rename: offered,
                truncate: { state: "absent" },
                drop: { state: "absent" },
              },
            },
            actions,
          },
        },
        connection: {
          connection: { state: { open: false, busy: false }, actions },
        },
        tab: {
          workspace: workspace(everything),
          tab: {
            state: { console: true, count: 3, toTheRight: 1, saved: true },
            actions,
          },
        },
        gridCell: {
          grid: {
            state: {
              target: "cell",
              origin: "preview",
              filterable: true,
              sortable: true,
              selectedRows: 2,
              oneColumn: true,
              relation: true,
              loadedRows: 10,
              shownColumns: 3,
              ai: { kind: "sampled" },
            },
            actions,
          },
        },
        gridHeader: {
          grid: {
            state: {
              target: "header",
              origin: "preview",
              filterable: true,
              sortable: true,
              selectedRows: 0,
              oneColumn: true,
              relation: true,
              loadedRows: 10,
              shownColumns: 3,
              ai: { kind: "none" },
            },
            actions,
          },
        },
        editor: {
          workspace: workspace(everything),
          editorMenu: {
            state: {
              readOnly: false,
              selection: true,
              objectUnderCursor: true,
            },
            actions,
          },
        },
        library: {
          libraryEntry: {
            state: { file: true, awaitingInspection: false },
            actions,
          },
        },
        assistantAnswer: {
          assistant: { state: { answering: false }, actions },
        },
        assistantQuestion: {
          assistant: { state: { answering: false }, actions },
        },
        assistantCode: {
          assistant: { state: { answering: false }, actions },
        },
        assistantMention: {
          assistant: { state: { answering: false }, actions },
        },
        erd: { erd: { state: {}, actions } },
      }) satisfies Record<Surface, ActionSources> as Record<
        Surface,
        ActionSources
      >
    const ids = (surface: Surface, sources: ActionSources) =>
      menuRows(surface, contextWith(sources))
        .flat()
        .flatMap((row) => (row.kind === "entry" ? [row.entry] : row.entries))
    const wired = targets(everything)
    const bare = targets({})
    for (const surface of Object.keys(CONTEXT_MENUS) as Array<Surface>) {
      const full = ids(surface, wired[surface])
      const unwired = ids(surface, bare[surface])
      expect(
        unwired.map((entry) => entry.id),
        surface
      ).toEqual(full.map((entry) => entry.id))
      for (const entry of unwired) {
        const before = full.find((candidate) => candidate.id === entry.id)
        if (before?.state === true) {
          expect(entry.state, `${surface}: ${entry.id}`).toHaveProperty(
            "reason"
          )
          // What is missing, and where it is done: never a bare refusal.
          const reason = entry.state === true ? "" : entry.state.reason
          expect(reason, `${surface}: ${entry.id}`).not.toMatch(GENERIC)
        }
      }
    }
  })

  it("keep no reason that leaves the user nowhere to go", () => {
    for (const [file, text] of Object.entries(REASON_SOURCES))
      expect(text, file).not.toMatch(GENERIC)
  })
})
