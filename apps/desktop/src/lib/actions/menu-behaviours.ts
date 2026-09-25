// The behaviours of the context menus' entries
// (docs/adr/0041-registre-d-actions-menus-et-raccourcis.md, point 6; UX-SPEC,
// « Menus contextuels »). Each acts on the target its menu put in the
// context (`menuContext`), through the handlers the surface gave — the same
// functions its buttons call (I-01).
//
// What these behaviours never do: compose SQL (the backend quotes, I-10),
// run what a model wrote (no `Run` on a code block, I-07), read a secret
// (I-03), send values to a model outside the sample approval (I-04), or copy
// more rows than one bounded read holds (I-06).

import { MAX_COPY_ROWS } from "@/components/oxyn/grid-selection"

import { consoleAvailability } from "./behaviour"
import type { ActionBehaviour, Availability } from "./behaviour"
import type { ActionContext, ActionSources } from "./context"
import type {
  CopyAsForm,
  CopyRowsFormat,
  GridMenuActions,
  GridMenuState,
  OperationKind,
} from "./targets"

type TargetKey =
  | "grid"
  | "tab"
  | "editorMenu"
  | "connection"
  | "libraryEntry"
  | "assistant"
  | "erd"
  | "catalogNode"
  | "objectOperation"

type SourceOf<TKey extends TargetKey> = NonNullable<ActionSources[TKey]>

/**
 * Why an entry is greyed on a surface that did not give its handler. Said by
 * surface, since what is missing is the surface's, not the target's.
 */
const NOT_WIRED: Record<TargetKey, string> = {
  grid: "Not available for this result",
  tab: "Not available for this tab",
  editorMenu: "Not available in this editor",
  connection: "Not available for this connection here",
  libraryEntry: "Not available for this entry",
  assistant: "Not available for this message",
  erd: "Not available in this diagram",
  catalogNode: "Not available in this catalog view",
  objectOperation: "Catalog operations are not available here",
}

const NOT_REWRITTEN = { reason: "The SQL you wrote is never rewritten" }
const COPY_LIMIT = MAX_COPY_ROWS.toLocaleString("en-US")

/**
 * An entry of the `key` surface running `pick`'s handler.
 *
 * Absent when the menu is not this surface's, or when `check` says the
 * target is not one the entry applies to — the cases UX-SPEC fixes (« Une
 * action, un libellé, un raccourci »). A handler the surface did not give
 * never hides an entry: it is greyed with what is missing, so two targets of
 * the same kind always offer the same menu.
 */
function onTarget<TKey extends TargetKey>(
  key: TKey,
  pick: (source: SourceOf<TKey>) => (() => void) | undefined,
  check: (
    state: SourceOf<TKey>["state"],
    context: ActionContext
  ) => Availability = () => true,
  missing: string = NOT_WIRED[key]
): ActionBehaviour {
  return {
    enabled: (context) => {
      const source = context.sources[key] as SourceOf<TKey> | undefined
      if (!source) return "absent"
      const state = check(source.state, context)
      if (state !== true) return state
      return pick(source) ? true : { reason: missing }
    },
    run: (context) => {
      const source = context.sources[key] as SourceOf<TKey> | undefined
      if (source) pick(source)?.()
    },
  }
}

/**
 * An entry the surface lists but Oxyn cannot do yet: greyed with what is
 * missing wherever its surface is, absent elsewhere. Each one is an écart of
 * the plan (« Interactions d'une application de bureau », lot 3).
 */
function notYet(key: TargetKey, reason: string): ActionBehaviour {
  return {
    enabled: (context) => (context.sources[key] ? { reason } : "absent"),
    run: () => undefined,
  }
}

/** A grid with no column looks empty: the last shown one stays. */
function lastColumnStays(state: GridMenuState): Availability {
  return state.shownColumns > 1
    ? true
    : { reason: "The last shown column stays" }
}

/** Filter and sort compose the preview's SQL; a console's is never touched. */
function previewShape(
  state: GridMenuState,
  declared: "filterable" | "sortable"
): Availability {
  if (state.origin === "query") return NOT_REWRITTEN
  return state[declared] ? true : "absent"
}

function copyRows(format: CopyRowsFormat): ActionBehaviour {
  return onTarget(
    "grid",
    (source) => {
      const copy = source.actions.copyRows
      return copy ? () => copy(format) : undefined
    },
    (state) => {
      if (state.selectedRows === 0) return { reason: "Nothing is selected" }
      if (state.selectedRows > MAX_COPY_ROWS)
        return {
          reason: `At most ${COPY_LIMIT} rows are copied at once: export the result`,
        }
      if (format === "insert" && !state.relation)
        return { reason: "These rows do not come from one known table" }
      if (format === "inList" && !state.oneColumn)
        return { reason: "Select cells of one column" }
      return true
    }
  )
}

/** Filtering by a value needs a bound value in the preview's filter. */
function valueFilter(): ActionBehaviour {
  return {
    enabled: (context) => {
      const grid = context.sources.grid
      if (!grid || grid.state.target !== "cell") return "absent"
      const shape = previewShape(grid.state, "filterable")
      if (shape !== true) return shape
      // TODO(2026-12-31, débloqué par la fusion du lot apercus-execution,
      // issues #14 et #16) — the preview's shape carries the user's predicate
      // as text only; a composed predicate needs a bound value beside it.
      return { reason: "Not available yet: the preview cannot bind a value" }
    },
    run: () => undefined,
  }
}

/**
 * An entry that shapes a preview's SQL. On a console's result it is greyed
 * whether or not the surface could act — the reason is the SQL, not the
 * surface —; on a preview it needs the source's declaration and a handler.
 */
function previewEntry(
  declared: "filterable" | "sortable",
  pick: (actions: GridMenuActions) => (() => void) | undefined
): ActionBehaviour {
  return {
    enabled: (context) => {
      const grid = context.sources.grid
      if (!grid) return "absent"
      const shape = previewShape(grid.state, declared)
      if (shape !== true) return shape
      return pick(grid.actions) ? true : { reason: NOT_WIRED.grid }
    },
    run: (context) => {
      const grid = context.sources.grid
      if (grid) pick(grid.actions)?.()
    },
  }
}

function sort(descending: boolean): ActionBehaviour {
  return previewEntry("sortable", (actions) => {
    const apply = actions.sort
    return apply ? () => apply(descending) : undefined
  })
}

function copyAs(form: CopyAsForm): ActionBehaviour {
  return onTarget(
    "catalogNode",
    (source) => {
      const copy = source.actions.copyAs
      return copy ? () => copy(form) : undefined
    },
    (state) => {
      if (!state.relation) return "absent"
      if (form === "ddl" && !state.definition)
        return { reason: "This session does not provide object definitions" }
      return true
    }
  )
}

/** Drop, Truncate and Rename open the in-place review; none runs (I-02). */
function operation(kind: OperationKind): ActionBehaviour {
  return onTarget(
    "objectOperation",
    (source) => {
      const review = source.actions.review
      return review ? () => review(kind) : undefined
    },
    (state) => {
      const offer = state.offers[kind]
      if (offer.state === "absent") return "absent"
      if (offer.state === "greyed") return { reason: offer.reason }
      return true
    }
  )
}

/** Cut, Copy and Paste of the editor's own menu. */
function editorClipboard(action: "cut" | "copy" | "paste"): ActionBehaviour {
  return onTarget(
    "editorMenu",
    (source) => source.actions[action],
    (state) => {
      if (action !== "copy" && state.readOnly)
        return { reason: "This editor is read-only" }
      if (action !== "paste" && !state.selection)
        return { reason: "Nothing is selected" }
      return true
    }
  )
}

/** Run selection and Run statement: absent from a read-only editor (UX-SPEC). */
function editorRun(
  pick: "runSelection" | "runStatement",
  needsSelection: boolean
): ActionBehaviour {
  return onTarget(
    "editorMenu",
    (source) => source.actions[pick],
    (state, context) => {
      if (state.readOnly) return "absent"
      if (needsSelection && !state.selection)
        return { reason: "Nothing is selected" }
      const console = context.sources.console
      return console
        ? consoleAvailability("run", console.state)
        : { reason: "No console is active" }
    }
  )
}

export const menuBehaviours: Record<string, ActionBehaviour> = {
  "tab.closeOthers": onTarget(
    "tab",
    (source) => source.actions.closeOthers,
    (state) => (state.count > 1 ? true : { reason: "No other tab is open" })
  ),
  "tab.closeRight": onTarget(
    "tab",
    (source) => source.actions.closeRight,
    (state) =>
      state.toTheRight > 0 ? true : { reason: "No tab is to the right" }
  ),
  "tab.closeAll": onTarget("tab", (source) => source.actions.closeAll),
  "tab.duplicate": onTarget(
    "tab",
    (source) => source.actions.duplicate,
    (state) =>
      state.console ? true : { reason: "Only a console is duplicated" }
  ),
  "tab.rename": onTarget(
    "tab",
    (source) => source.actions.rename,
    (state) => (state.console ? true : { reason: "Only a console is renamed" })
  ),
  "tab.openInNewWindow": notYet(
    "tab",
    "Not available yet: Oxyn opens a single window"
  ),
  "tab.revealInLibrary": onTarget(
    "tab",
    (source) => source.actions.revealInLibrary,
    (state) => {
      if (!state.console)
        return { reason: "Only a console is saved in the library" }
      return state.saved
        ? true
        : { reason: "This console is not saved in the library" }
    }
  ),

  "grid.copyValue": onTarget(
    "grid",
    (source) => source.actions.copyValue,
    (state) => (state.target === "cell" ? true : "absent")
  ),
  "grid.copyRows.tsv": copyRows("tsv"),
  "grid.copyRows.csv": copyRows("csv"),
  "grid.copyRows.json": copyRows("json"),
  "grid.copyRows.markdown": copyRows("markdown"),
  "grid.copyRows.insert": copyRows("insert"),
  "grid.copyRows.inList": copyRows("inList"),
  "grid.inspectValue": onTarget("grid", (source) => source.actions.inspect),
  "grid.filterByValue": valueFilter(),
  "grid.excludeValue": valueFilter(),
  "grid.filterNull": valueFilter(),
  "grid.sortAscending": sort(false),
  "grid.sortDescending": sort(true),
  "grid.hideColumn": onTarget(
    "grid",
    (source) => source.actions.hideColumn,
    lastColumnStays
  ),
  "grid.openReferencedRow": notYet(
    "grid",
    "Not available yet: the grid does not know which key a column belongs to"
  ),
  // Values reach a model only through the sample approval, under `Sampled`
  // (I-04); without a declared destination the entry does not exist.
  "grid.sendToAssistant": {
    enabled: (context) => {
      const grid = context.sources.grid
      if (!grid) return "absent"
      const ai = grid.state.ai
      if (ai.kind === "none") return "absent"
      if (ai.kind === "withheld")
        return { reason: `The connection's AI level is ${ai.label}` }
      if (!grid.actions.sendToAssistant)
        return {
          reason:
            "Not available yet: a selection cannot open the sample approval",
        }
      return true
    },
    run: (context) => context.sources.grid?.actions.sendToAssistant?.(),
  },
  "column.filter": previewEntry("filterable", (actions) => actions.filter),
  "column.hide": onTarget(
    "grid",
    (source) => source.actions.hideColumn,
    lastColumnStays
  ),
  "column.freeze": notYet(
    "grid",
    "Not available yet: the grid cannot freeze a column"
  ),
  "column.autosize": onTarget("grid", (source) => source.actions.autosize),
  "column.copyName": onTarget("grid", (source) => source.actions.copyName),
  // The loaded rows only, never the column: the rest is not in memory, and
  // is not read for a copy (I-06).
  "column.copyValues": onTarget(
    "grid",
    (source) => source.actions.copyValues,
    (state) => {
      if (state.loadedRows === 0) return { reason: "No row is loaded" }
      if (state.loadedRows > MAX_COPY_ROWS)
        return {
          reason: `More than ${COPY_LIMIT} loaded rows: export the result`,
        }
      return true
    }
  ),

  "editor.runSelection": editorRun("runSelection", true),
  "editor.runStatement": editorRun("runStatement", false),
  "editor.format": notYet(
    "editorMenu",
    "Not available yet: Oxyn has no SQL formatter"
  ),
  "editor.openObject": onTarget(
    "editorMenu",
    (source) => source.actions.openObject,
    (state) =>
      state.objectUnderCursor
        ? true
        : { reason: "No loaded object is named under the cursor" }
  ),
  // Absent only without an AI destination (UX-SPEC); an editor whose host
  // cannot hand text to the composer greys it with what is missing.
  "editor.askAssistant": {
    enabled: (context) => {
      const editor = context.sources.editorMenu
      if (!editor || !context.sources.workspace?.state.hasAssistant)
        return "absent"
      if (!editor.actions.askAssistant)
        return {
          reason:
            "Not available yet: the assistant's composer cannot receive text",
        }
      return editor.state.selection ? true : { reason: "Nothing is selected" }
    },
    run: (context) => context.sources.editorMenu?.actions.askAssistant?.(),
  },

  "connection.connect": onTarget(
    "connection",
    (source) => source.actions.connect,
    (state) => {
      if (state.open) return "absent"
      return state.busy ? { reason: "The connection is busy" } : true
    }
  ),
  "connection.disconnect": onTarget(
    "connection",
    (source) => source.actions.disconnect,
    (state) => {
      if (!state.open) return "absent"
      return state.busy ? { reason: "The connection is busy" } : true
    }
  ),
  "connection.newConsole": onTarget(
    "connection",
    (source) => source.actions.newConsole,
    (state) => (state.busy ? { reason: "The connection is busy" } : true)
  ),
  "connection.refreshCatalog": onTarget(
    "connection",
    (source) => source.actions.refreshCatalog,
    (state) => (state.open ? true : { reason: "Connect to read its catalog" })
  ),
  "connection.edit": onTarget("connection", (source) => source.actions.edit),
  "connection.duplicate": onTarget(
    "connection",
    (source) => source.actions.duplicate
  ),
  "connection.changeEnvironment": onTarget(
    "connection",
    (source) => source.actions.changeEnvironment
  ),
  // What the front already holds, never a secret (I-03): the parameters and
  // the secret's reference do not cross the IPC.
  "connection.copy": onTarget("connection", (source) => source.actions.copy),
  "connection.delete": onTarget(
    "connection",
    (source) => source.actions.delete,
    (state) => {
      // As the button: the connection in use is left first.
      if (state.open) return { reason: "Disconnect it to delete it" }
      return state.busy ? { reason: "The connection is busy" } : true
    }
  ),

  "library.openEntry": onTarget(
    "libraryEntry",
    (source) => source.actions.open,
    (state) =>
      state.awaitingInspection
        ? { reason: "A write awaiting inspection opens no editable copy" }
        : true
  ),
  "library.rename": onTarget(
    "libraryEntry",
    (source) => source.actions.rename,
    // A history row is no file: there is nothing to rename or point at.
    (state) => (state.file ? true : "absent"),
    "Not available yet: a saved query cannot be renamed in one step"
  ),
  "library.duplicate": onTarget(
    "libraryEntry",
    (source) => source.actions.duplicate,
    (state) => (state.file ? true : "absent"),
    "Not available yet: a saved query cannot be duplicated in one step"
  ),
  // Only a saved query is a file to reveal; a history row has none.
  "library.reveal": {
    enabled: (context) =>
      context.sources.libraryEntry?.state.file
        ? { reason: "Not available yet: Oxyn cannot open the file manager" }
        : "absent",
    run: () => undefined,
  },
  "library.copyPath": onTarget(
    "libraryEntry",
    (source) => source.actions.copyPath,
    (state) => (state.file ? true : "absent"),
    "Not available yet: the library does not give the file's path"
  ),
  "library.delete": onTarget("libraryEntry", (source) => source.actions.delete),

  "assistant.copyAnswer": onTarget(
    "assistant",
    (source) => source.actions.copyAnswer
  ),
  "assistant.copyMarkdown": onTarget(
    "assistant",
    (source) => source.actions.copyMarkdown
  ),
  "assistant.regenerate": onTarget(
    "assistant",
    (source) => source.actions.regenerate,
    (state) =>
      state.answering ? { reason: "An answer is being written" } : true
  ),
  "assistant.editQuestion": onTarget(
    "assistant",
    (source) => source.actions.editQuestion,
    (state) =>
      state.answering ? { reason: "An answer is being written" } : true
  ),
  // A code block is copied or put in a console; it never runs from here (I-07).
  "assistant.copyCode": onTarget(
    "assistant",
    (source) => source.actions.copyCode
  ),
  "assistant.openInConsole": onTarget(
    "assistant",
    (source) => source.actions.openInConsole,
    (state) => state.openSql ?? true
  ),
  "assistant.openObject": onTarget(
    "assistant",
    (source) => source.actions.openObject
  ),

  "erd.openTable": onTarget("erd", (source) => source.actions.openTable),
  "erd.copyName": onTarget("erd", (source) => source.actions.copyName),
  "erd.relayout": notYet(
    "erd",
    "Not available yet: the diagram is laid out once"
  ),
  "erd.exportImage": notYet(
    "erd",
    "Not available yet: Oxyn cannot write an image file"
  ),

  "catalog.openData": onTarget(
    "catalogNode",
    (source) => source.actions.openData,
    (state) => {
      if (!state.relation) return "absent"
      return state.holdsRecords ? true : { reason: "This object holds no rows" }
    }
  ),
  "catalog.viewStructure": onTarget(
    "catalogNode",
    (source) => source.actions.viewStructure,
    (state) => (state.relation ? true : "absent")
  ),
  "catalog.viewDdl": onTarget(
    "catalogNode",
    (source) => source.actions.viewDdl,
    (state) => (state.relation ? true : "absent")
  ),
  // Only where a console's session context can be this schema (UX-SPEC,
  // « Contexte de session d'une console »).
  "catalog.newConsoleOnSchema": onTarget(
    "catalogNode",
    (source) => source.actions.newConsoleOnSchema,
    (state) => (state.schemaContext ? true : "absent")
  ),
  "catalog.copyQualifiedName": onTarget(
    "catalogNode",
    (source) => source.actions.copyQualifiedName,
    (state) => (state.relation ? true : "absent")
  ),
  "catalog.copyAs.quotedName": copyAs("quotedName"),
  "catalog.copyAs.selectAll": copyAs("selectAll"),
  "catalog.copyAs.insertTemplate": copyAs("insertTemplate"),
  "catalog.copyAs.ddl": copyAs("ddl"),
  "catalog.refresh": onTarget(
    "catalogNode",
    (source) => source.actions.refresh,
    (state) => (state.relation ? "absent" : true)
  ),
  "catalog.collapseAll": onTarget(
    "catalogNode",
    (source) => source.actions.collapseAll,
    (state) => (state.expanded ? true : { reason: "Nothing is expanded" })
  ),
  "catalog.pin": onTarget(
    "catalogNode",
    (source) => source.actions.pin,
    (state) => state.pin
  ),
  "object.rename": operation("rename"),
  "object.truncate": operation("truncate"),
  "object.drop": operation("drop"),
}

/** Cut, Copy and Paste defer to the editor's menu when it is the target. */
export const editorClipboardBehaviours = {
  cut: editorClipboard("cut"),
  copy: editorClipboard("copy"),
  paste: editorClipboard("paste"),
}
