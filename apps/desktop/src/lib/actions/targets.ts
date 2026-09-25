// What a context menu acts on (docs/adr/0041-registre-d-actions-menus-et-raccourcis.md,
// point 6): the target of a right click, as a source of the `ActionContext`.
//
// A surface never publishes its target for good: its menu builds a context
// with the target for the time it is open (`menuContext`), and every entry is
// evaluated and invoked on that context. What the state says decides whether
// an entry applies to the target — absent only in the cases UX-SPEC fixes —
// and greys it; a handler the surface leaves out greys it too, never hides it.

/** Where the rows of a grid come from. */
export type GridOrigin =
  /** A table preview: Oxyn composes its SQL, filter and sort included. */
  | "preview"
  /** A console or a retained result: the SQL the user wrote, never rewritten. */
  | "query"

export type CopyRowsFormat =
  "tsv" | "csv" | "json" | "markdown" | "insert" | "inList"

/** The AI level of the connection, as the grid entries read it (I-04). */
export type GridAiLevel =
  /** No AI destination is declared: the entry does not exist. */
  | { kind: "none" }
  /** The level lets values reach the model through the sample approval. */
  | { kind: "sampled" }
  /** Any other level, named in the greyed entry's reason. */
  | { kind: "withheld"; label: string }

export interface GridMenuState {
  /** What was right-clicked: a cell (or the selection around it), or a header. */
  target: "cell" | "header"
  origin: GridOrigin
  /** The source declares a filter for its preview; else the entries are absent. */
  filterable: boolean
  /** The source declares an order for its preview; else the entries are absent. */
  sortable: boolean
  /** Rows of the selection a copy would carry. */
  selectedRows: number
  /** The selection lies in one column (`IN list`). */
  oneColumn: boolean
  /** The rows come from one known relation (`INSERT`). */
  relation: boolean
  /** Rows the result holds now, for `Copy values` (I-06). */
  loadedRows: number
  /** Columns shown: the last one is never hidden. */
  shownColumns: number
  ai: GridAiLevel
}

export interface GridMenuActions {
  copyValue?: () => void
  copyRows?: (format: CopyRowsFormat) => void
  inspect?: () => void
  sort?: (descending: boolean) => void
  hideColumn?: () => void
  sendToAssistant?: () => void
  filter?: () => void
  autosize?: () => void
  copyName?: () => void
  copyValues?: () => void
}

export interface TabMenuState {
  /** The tab is a console: only a console is duplicated or renamed. */
  console: boolean
  /** Tabs open, this one included. */
  count: number
  /** Tabs after this one. */
  toTheRight: number
  /** The console is saved in the library. */
  saved: boolean
}

export interface TabMenuActions {
  close?: () => void
  closeOthers?: () => void
  closeRight?: () => void
  closeAll?: () => void
  duplicate?: () => void
  rename?: () => void
  revealInLibrary?: () => void
}

export interface EditorMenuState {
  /** A stored query's read-only view: nothing runs from it. */
  readOnly: boolean
  selection: boolean
  /** A name the loaded catalog resolves sits under the cursor. */
  objectUnderCursor: boolean
}

export interface EditorMenuActions {
  cut?: () => void
  copy?: () => void
  paste?: () => void
  runSelection?: () => void
  runStatement?: () => void
  openObject?: () => void
  askAssistant?: () => void
}

export interface ConnectionMenuState {
  /** The workspace of this connection is open. */
  open: boolean
  /** Opening, closing or deleting is under way. */
  busy: boolean
}

export interface ConnectionMenuActions {
  connect?: () => void
  disconnect?: () => void
  newConsole?: () => void
  refreshCatalog?: () => void
  edit?: () => void
  duplicate?: () => void
  changeEnvironment?: () => void
  copy?: () => void
  delete?: () => void
}

export interface LibraryMenuState {
  /** The entry is a file on disk (a saved query), with a path. */
  file: boolean
  /** A history write marked for inspection: it opens no editable copy. */
  awaitingInspection: boolean
}

export interface LibraryMenuActions {
  open?: () => void
  rename?: () => void
  duplicate?: () => void
  copyPath?: () => void
  delete?: () => void
}

export interface AssistantMenuState {
  /** An answer is being written: it cannot be regenerated yet. */
  answering: boolean
  /**
   * `Open in console` on a code block: absent from a block that is not SQL,
   * greyed with the reason its button gives. Only a code block sets it.
   */
  openSql?: true | { reason: string } | "absent"
}

export interface AssistantMenuActions {
  copyAnswer?: () => void
  copyMarkdown?: () => void
  regenerate?: () => void
  editQuestion?: () => void
  copyCode?: () => void
  openInConsole?: () => void
  openObject?: () => void
}

export type ErdMenuState = Record<string, never>

export interface ErdMenuActions {
  openTable?: () => void
  copyName?: () => void
}

export type CopyAsForm = "quotedName" | "selectAll" | "insertTemplate" | "ddl"

export interface CatalogMenuState {
  /** The node is a relation: open, describe and copy it. */
  relation: boolean
  /** The relation holds records (`Open data`). */
  holdsRecords: boolean
  /** The node is a schema whose session context a console can take. */
  schemaContext: boolean
  /** The relation's definition can be read (`Copy as ▸ DDL`). */
  definition: boolean
  /** Some level of the tree is expanded. */
  expanded: boolean
  /** `Pin to question` on this node (`pinOffer`). */
  pin: true | { reason: string } | "absent"
}

export interface CatalogMenuActions {
  openData?: () => void
  viewStructure?: () => void
  viewDdl?: () => void
  newConsoleOnSchema?: () => void
  copyQualifiedName?: () => void
  copyAs?: (form: CopyAsForm) => void
  refresh?: () => void
  collapseAll?: () => void
  pin?: () => void
}

export type OperationKind = "rename" | "truncate" | "drop"

/** An operation the target's session offers, greys, or does not offer (ADR-0042). */
export type OperationOffer =
  | { state: "offered" }
  | { state: "greyed"; reason: string }
  | { state: "absent" }

export interface ObjectOperationState {
  offers: Record<OperationKind, OperationOffer>
}

export interface ObjectOperationActions {
  /** Opens the in-place review; nothing runs from the menu (I-02). */
  review?: (kind: OperationKind) => void
}
