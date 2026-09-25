import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import { ViewIcon } from "@hugeicons/core-free-icons"
import { useStore } from "@tanstack/react-store"

import { addressKey } from "@/components/oxyn/catalog-tree"
import type { OpenTarget } from "@/components/oxyn/catalog-tree"
import { ExportMenuView, ExportSubmenu } from "@/components/oxyn/export-menu"
import { FacetFrame } from "@/components/oxyn/facet-frame"
import {
  ObjectViewFrame,
  PreviewToolbar,
  RelationsPanel,
  initialObjectTab,
} from "@/components/oxyn/object-view-frame"
import type { ObjectTab } from "@/components/oxyn/object-view-frame"
import { PreviewControls } from "@/components/oxyn/preview-controls"
import { RelationConstraints } from "@/components/oxyn/relation-constraints"
import {
  DefinitionActions,
  RelationDefinition,
} from "@/components/oxyn/relation-definition"
import { RelationIndexes } from "@/components/oxyn/relation-indexes"
import { IncomingKeys, OutgoingKeys } from "@/components/oxyn/relation-keys"
import { operationOffer } from "@/components/oxyn/object-operations"
import { RelationStructure } from "@/components/oxyn/relation-structure"
import { RestoredObjectNotice } from "@/components/oxyn/restored-object-notice"
import { ResultPanel } from "@/components/oxyn/result-panel"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { DropdownMenuItem } from "@/components/ui/dropdown-menu"
import {
  previewUnavailable,
  unsupportedReason,
} from "@/features/metadata/capabilities"
import { copyToClipboard } from "@/features/metadata/clipboard"
import { inspectObject, inspection } from "@/features/metadata/inspection"
import { useObjectOperation } from "@/features/metadata/object-operation-flow"
import { usePreview } from "@/features/metadata/use-preview"
import { useRelationFacets } from "@/features/metadata/use-relation-facets"
import {
  ValueInspectionDialog,
  gridInspection,
  useReleaseSelection,
} from "@/features/metadata/value-inspection"
import { useResultDensity } from "@/features/settings/use-result-density"
import { useResultExport } from "@/features/workspace/export-menu"
import { useGridMenu } from "@/features/workspace/grid-menu"
import { sectionOf, tabOf } from "@/features/workspace/object-location"
import type { RelationDirection } from "@/features/workspace/object-location"
import { useCompact } from "@/features/workspace/use-compact"
import type { SectionChoice } from "@/lib/ipc/location"
import { metadata } from "@/lib/ipc/metadata"
import type { BoundValue } from "@/lib/ipc/metadata"
import { results } from "@/lib/ipc/results"
import type {
  CatalogAddress,
  CatalogNode,
  OpenConnection,
} from "@/lib/ipc/types"

/** What the workspace asks of the object on screen. */
export type ObjectViewHandle = {
  /** Shows Data and focuses its grid (`⌘2`); nothing without a preview. */
  focusPreview: () => void
}

/**
 * A selected table or view: its rows, structure, indexes, constraints,
 * relations and definition, each loaded when its tab is opened.
 *
 * The preview has its own grid and its own cancellation; it replaces neither
 * the SQL draft nor the console's result (docs/UX-SPEC.md, « Données d'une
 * table sélectionnée »). It is kept once read: coming back to this object
 * shows it again without a new read. Nothing here runs SQL the user did not
 * write: the definition and the related-row query open in a console, where
 * the policy applies to whatever is run.
 */
export function ObjectView({
  open,
  node,
  active,
  initialTab,
  restored,
  onPlaceChange,
  onInspectRow,
  onOpenRelated,
  onOpenInConsole,
  handleRef,
  definitionWidth,
  onDefinitionWidthChange,
}: {
  open: OpenConnection
  node: CatalogNode
  /**
   * On screen: the active tab of a visible workspace. Kept mounted behind
   * another tab, this view reads nothing; a change marks its preview stale,
   * read once when it shows again (ADR-0022).
   */
  active: boolean
  initialTab?: OpenTarget
  /**
   * The sub-view saved in the last session. The tab opens on it and reads
   * nothing — rows or metadata — until the user asks.
   */
  restored?: SectionChoice
  /** The sub-view shown, each time it changes: saved for the next launch. */
  onPlaceChange?: (section: SectionChoice) => void
  /** Shows the selected row's inspector, overlaid at compact width. */
  onInspectRow?: () => void
  /** Selects another relation, as a click in the explorer would. */
  onOpenRelated?: (address: CatalogAddress) => void
  /**
   * Opens SQL in a **new** console, without running it. `parameters`
   * pre-fills its Parameters panel; the values are bound there, never
   * written into the text.
   */
  onOpenInConsole?: (
    sql: string,
    title: string,
    options?: {
      parameters?: Array<BoundValue>
      /** A value is still missing: the console says so before running. */
      needsValues?: boolean
      /** Said by the console above the text, before anything is run. */
      notice?: string
    }
  ) => void
  handleRef?: React.Ref<ObjectViewHandle>
  /** The definition panel's width, kept by the workspace (wide layout). */
  definitionWidth: number
  onDefinitionWidthChange: (width: number) => void
}) {
  const dataUnavailable = previewUnavailable(open, node.holdsRecords)
  const previewable = dataUnavailable === null
  const compact = useCompact()
  // `Rename…` of a column, from the Structure tab's menu (ADR-0042).
  const columnOperation = useObjectOperation(open, () => undefined)
  const restoredAt = restored ? tabOf(restored) : null
  const [chosenTab, setTab] = React.useState<ObjectTab>(() =>
    restoredAt && !(restoredAt.tab === "data" && !previewable)
      ? restoredAt.tab
      : initialObjectTab(initialTab, previewable)
  )
  // Wide, the definition sits beside the metadata instead of being a tab: the
  // choice is kept for the compact layout, which lists it again.
  const tab: ObjectTab =
    !compact && chosenTab === "definition" ? "structure" : chosenTab
  // A restored tab waits for a gesture: choosing a view, Refresh, Read it now.
  const [held, setHeld] = React.useState(restored !== undefined)
  const [gridFocusRequest, setGridFocusRequest] = React.useState(0)
  React.useImperativeHandle(
    handleRef,
    () => ({
      focusPreview: () => {
        if (!previewable) return
        setHeld(false)
        setTab("data")
        setGridFocusRequest((count) => count + 1)
      },
    }),
    [previewable]
  )
  const [direction, setDirection] = React.useState<RelationDirection>(
    () =>
      restoredAt?.direction ??
      (unsupportedReason(open, "incoming") ? "outgoing" : "incoming")
  )
  const source = `preview:${open.connection}:${addressKey(node.address)}`
  const facets = useRelationFacets(open, node.address)
  const preview = usePreview({
    open,
    address: node.address,
    enabled: previewable,
    visible: active && tab === "data" && !held,
  })

  const placeChange = React.useRef(onPlaceChange)
  placeChange.current = onPlaceChange
  // Held, the saved place is left as it was: only a gesture moves it.
  React.useEffect(() => {
    // The tab chosen, not the one drawn: wide, a chosen DDL shows Structure
    // beside the definition, and is still DDL once narrowed.
    if (!held) placeChange.current?.(sectionOf(chosenTab, direction))
  }, [held, chosenTab, direction])
  const chooseTab = (next: ObjectTab) => {
    setHeld(false)
    setTab(next)
  }
  const chooseDirection = (next: RelationDirection) => {
    setHeld(false)
    setDirection(next)
  }

  React.useEffect(() => {
    inspectObject({ connection: open.connection, address: node.address })
    return () => {
      const shown = inspection.state.object
      if (
        shown?.connection === open.connection &&
        addressKey(shown.address) === addressKey(node.address)
      )
        inspectObject(null)
    }
  }, [open.connection, node.address])
  useReleaseSelection(source)

  const { ensure } = facets
  React.useEffect(() => {
    if (held) return
    switch (tab) {
      case "structure":
      case "indexes":
        ensure("detail")
        break
      case "constraints":
        if (!unsupportedReason(open, "constraints")) ensure("constraints")
        break
      case "relations":
        if (direction === "incoming") {
          if (!unsupportedReason(open, "incoming")) ensure("incomingKeys")
        } else ensure("detail")
        break
      case "definition":
        if (!unsupportedReason(open, "definition")) ensure("definition")
        break
      default:
        break
    }
    // Asked once, like a tab: a window resized across 1200 px reads nothing.
    if (!compact && tab !== "data" && !unsupportedReason(open, "definition"))
      ensure("definition")
  }, [held, tab, direction, compact, ensure])

  const data = facets.facets
  const state = preview.state
  const result = state.status === "populated" ? state.result : null
  const shownResult =
    state.status === "populated"
      ? state.result
      : state.status === "running"
        ? (state.result ?? null)
        : null
  const exportable =
    state.status === "populated" &&
    state.complete &&
    !state.truncated &&
    !state.cancelled
  const fetchPage = React.useCallback(
    (offset: number, limit: number) =>
      shownResult
        ? results.readResultPage(open.connection, shownResult, offset, limit)
        : Promise.reject(new Error("No preview is held.")),
    [open.connection, shownResult]
  )

  // The row selected in this preview, when there is one: its key values are
  // read in Rust from the result buffer, never from the formatted cells.
  const selected = useStore(inspection, (inspected) =>
    inspected.selectedRow?.source === source ? inspected.selectedRow : null
  )
  const reviewRelatedRows = onOpenInConsole
    ? async (incoming: boolean, index: number) => {
        const query = await metadata.relatedRowsTemplate(
          open.connection,
          node.address,
          incoming,
          index,
          selected ? { result: selected.result, row: selected.row } : null
        )
        if (query)
          onOpenInConsole(query.sql, "Related rows.sql", {
            parameters: query.parameters,
            needsValues: query.needsValues,
          })
      }
    : undefined

  const detailLoad = facets.loads.detail
  const definitionStale =
    data?.definition.freshness.state === "invalidated" ||
    facets.loads.definition.status === "error"
  // Restored, then read again from the server, and the relation was not
  // there: said, and its place kept (docs/UX-SPEC.md).
  const vanished =
    restored !== undefined &&
    !held &&
    data?.detail.freshness.state === "fetched" &&
    data.detail.value === null

  const density = useResultDensity()
  const filterRef = React.useRef<HTMLInputElement>(null)
  const { applySort } = preview
  const gridMenu = useGridMenu({
    open,
    connection: open.connection,
    result: shownResult,
    origin: "preview",
    address: node.address,
    filterable: open.capabilities.includes("PREVIEW_FILTER"),
    sortable: open.capabilities.includes("PREVIEW_SORT"),
    sort: React.useCallback(
      (column: string, descending: boolean) =>
        applySort([{ column, descending }]),
      [applySort]
    ),
    filter: React.useCallback(() => filterRef.current?.focus(), []),
  })
  const exporter = useResultExport({
    connection: open.connection,
    result,
    defaultName: node.name,
  })
  const exportChoice = {
    formats: exporter.formats,
    formatsFailed: exporter.formatsFailed,
    exportable: exportable && result !== null,
    reason:
      state.status === "populated"
        ? "This preview is partial: it was cancelled or truncated."
        : "Export becomes available once the preview has finished loading.",
    label: "Export preview…",
    scope:
      state.status === "populated"
        ? `The ${state.rows.toLocaleString("en-US")} preview rows shown · not the entire table`
        : "The preview rows shown · not the entire table",
  }
  const exportMenu = (
    <ExportMenuView
      {...exportChoice}
      state={exporter.state}
      onExport={exporter.run}
      onCancel={exporter.cancel}
    />
  )

  return (
    <ObjectViewFrame
      name={node.name}
      kind={data?.kind ?? node.kind}
      tab={tab}
      onTabChange={chooseTab}
      compact={compact}
      dataUnavailable={dataUnavailable}
      onEscape={tab === "data" && preview.running ? preview.cancel : undefined}
      gridFocusRequest={gridFocusRequest}
      definitionLayout={{
        beside: !compact,
        width: definitionWidth,
        onWidthChange: onDefinitionWidthChange,
      }}
      toolbar={
        tab === "data" && previewable ? (
          <PreviewToolbar
            running={preview.running}
            cancelling={preview.cancelling}
            onRefresh={() => {
              setHeld(false)
              preview.refresh()
            }}
            onCancel={preview.cancel}
          />
        ) : null
      }
      notice={
        <>
          {held ? (
            <RestoredObjectNotice
              state="held"
              name={node.name}
              onRead={() => setHeld(false)}
            />
          ) : vanished ? (
            <RestoredObjectNotice state="vanished" name={node.name} />
          ) : null}
          {facets.error ? (
            <Alert
              variant="destructive"
              className="rounded-none border-x-0 border-t-0"
            >
              <AlertTitle>The catalog cache could not be read</AlertTitle>
              <AlertDescription className="font-mono text-xs text-foreground">
                {facets.error}
              </AlertDescription>
            </Alert>
          ) : null}
        </>
      }
      panels={{
        data: previewable ? (
          <>
            <PreviewControls
              canFilter={open.capabilities.includes("PREVIEW_FILTER")}
              canSort={open.capabilities.includes("PREVIEW_SORT")}
              columns={
                data?.detail.value?.fields.map((field) => field.name) ?? null
              }
              applied={preview.applied}
              status={preview.status}
              pagination={preview.pagination}
              loadingColumns={detailLoad.status === "loading"}
              onApplyPredicate={preview.applyPredicate}
              onApplySort={preview.applySort}
              onPage={preview.page}
              onCancel={preview.cancel}
              onLoadColumns={() => void facets.refresh("detail")}
              filterRef={filterRef}
            />
            {preview.approvalRefused ? (
              <Alert className="rounded-none border-x-0 border-t-0 py-2">
                <AlertTitle>Preview not read</AlertTitle>
                <AlertDescription>
                  This connection&apos;s policy requires an explicit approval
                  for this read. Run it from a SQL console instead.
                </AlertDescription>
              </Alert>
            ) : null}
            <div className="min-h-0 flex-1">
              <ResultPanel
                state={state}
                initialHint="Refresh data reads the first rows of this object."
                fetchPage={fetchPage}
                onCancel={preview.cancel}
                onRetry={preview.refresh}
                context={{ connectionName: open.name, statement: null }}
                footerNote="Preview · total row count not requested"
                density={density}
                gridMenu={gridMenu}
                // Compact: the export sits in `Actions`, and the footer shows
                // only a running export's progress and its Cancel.
                footerActions={
                  !compact || exporter.state.status === "exporting"
                    ? exportMenu
                    : null
                }
                compactActions={
                  compact ? (
                    <>
                      <ExportSubmenu
                        {...exportChoice}
                        exporting={exporter.state.status === "exporting"}
                        onExport={exporter.run}
                      />
                      <DropdownMenuItem
                        disabled={!selected || !onInspectRow}
                        onClick={onInspectRow}
                      >
                        <HugeiconsIcon icon={ViewIcon} strokeWidth={2} />
                        Inspect row
                      </DropdownMenuItem>
                    </>
                  ) : undefined
                }
                {...gridInspection({
                  source,
                  open,
                  result,
                  columns: state.status === "populated" ? state.columns : [],
                })}
              />
            </div>
            <ValueInspectionDialog source={source} />
          </>
        ) : null,

        structure: data ? (
          <FacetFrame
            label="columns"
            freshness={data.detail.freshness}
            load={detailLoad}
            unsupported={null}
            hasValue={data.detail.value !== null}
            empty={data.detail.value?.fields.length === 0}
            emptyText="No columns reported for this object."
            onRefresh={() => void facets.refresh("detail")}
            onCancel={() => facets.cancel("detail")}
          >
            <RelationStructure
              detail={data.detail.value}
              renameColumn={{
                offer: operationOffer("rename", node, open.capabilities),
                onRename: (column) =>
                  columnOperation.start(node, "rename", column),
              }}
            />
            {/* Modal while open: no tab change can unmount it mid-review. */}
            {columnOperation.element}
          </FacetFrame>
        ) : (
          <RelationStructure
            detail={undefined}
            error={
              facets.error ? { message: facets.error, retryable: false } : null
            }
            onRefresh={() => void facets.refresh("detail")}
            refreshing={detailLoad.status === "loading"}
          />
        ),

        indexes: data ? (
          <FacetFrame
            label="indexes"
            freshness={data.detail.freshness}
            load={detailLoad}
            unsupported={unsupportedReason(open, "indexes")}
            hasValue={data.indexes !== null}
            empty={data.indexes?.length === 0}
            emptyText="No indexes reported for this object."
            onRefresh={() => void facets.refresh("detail")}
            onCancel={() => facets.cancel("detail")}
          >
            <RelationIndexes indexes={data.indexes ?? []} />
          </FacetFrame>
        ) : null,

        constraints: data ? (
          <FacetFrame
            label="constraints"
            freshness={data.constraints.freshness}
            load={facets.loads.constraints}
            unsupported={unsupportedReason(open, "constraints")}
            hasValue={data.constraints.value !== null}
            empty={data.constraints.value?.length === 0}
            emptyText="No constraints reported for this object."
            onRefresh={() => void facets.refresh("constraints")}
            onCancel={() => facets.cancel("constraints")}
          >
            <RelationConstraints
              constraints={data.constraints.value ?? []}
              detail={data.detail.value}
              indexes={data.indexes}
              onCopyDefinition={(text) =>
                void copyToClipboard(text, "Constraint definition")
              }
            />
          </FacetFrame>
        ) : null,

        relations: (
          <RelationsPanel
            direction={direction}
            onDirectionChange={chooseDirection}
          >
            {data && direction === "outgoing" ? (
              <FacetFrame
                label="foreign keys"
                freshness={data.detail.freshness}
                load={detailLoad}
                unsupported={unsupportedReason(open, "relations")}
                hasValue={data.foreignKeys !== null}
                empty={data.foreignKeys?.length === 0}
                emptyText="No outgoing foreign keys reported for this object."
                onRefresh={() => void facets.refresh("detail")}
                onCancel={() => facets.cancel("detail")}
              >
                <OutgoingKeys
                  keys={data.foreignKeys ?? []}
                  onOpen={onOpenRelated}
                  onReviewRelatedRows={
                    reviewRelatedRows
                      ? (index) => void reviewRelatedRows(false, index)
                      : undefined
                  }
                />
              </FacetFrame>
            ) : null}
            {data && direction === "incoming" ? (
              <FacetFrame
                label="incoming foreign keys"
                freshness={data.incomingKeys.freshness}
                load={facets.loads.incomingKeys}
                unsupported={unsupportedReason(open, "incoming")}
                hasValue={data.incomingKeys.value !== null}
                empty={data.incomingKeys.value?.length === 0}
                emptyText="No incoming foreign keys reported for this object."
                onRefresh={() => void facets.refresh("incomingKeys")}
                onCancel={() => facets.cancel("incomingKeys")}
              >
                <IncomingKeys
                  keys={data.incomingKeys.value ?? []}
                  onOpen={onOpenRelated}
                  onReviewRelatedRows={
                    reviewRelatedRows
                      ? (index) => void reviewRelatedRows(true, index)
                      : undefined
                  }
                />
              </FacetFrame>
            ) : null}
          </RelationsPanel>
        ),

        definition: data ? (
          <FacetFrame
            label="the definition"
            freshness={data.definition.freshness}
            load={facets.loads.definition}
            unsupported={unsupportedReason(open, "definition")}
            hasValue={data.definition.value !== null}
            empty={false}
            emptyText=""
            onRefresh={() => void facets.refresh("definition")}
            onCancel={() => facets.cancel("definition")}
            actions={
              data.definition.value ? (
                <DefinitionActions
                  definition={data.definition.value}
                  stale={definitionStale}
                  onCopy={(sql) => void copyToClipboard(sql, "DDL")}
                  onOpenInConsole={
                    onOpenInConsole
                      ? (sql, notice) =>
                          onOpenInConsole(sql, "Object DDL", { notice })
                      : undefined
                  }
                />
              ) : null
            }
          >
            {data.definition.value ? (
              <RelationDefinition
                definition={data.definition.value}
                stale={definitionStale}
              />
            ) : null}
          </FacetFrame>
        ) : null,
      }}
    />
  )
}
