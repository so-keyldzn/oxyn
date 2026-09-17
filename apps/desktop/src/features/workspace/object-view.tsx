import * as React from "react"
import { useStore } from "@tanstack/react-store"

import { addressKey } from "@/components/oxyn/catalog-tree"
import type { OpenTarget } from "@/components/oxyn/catalog-tree"
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
import { RelationStructure } from "@/components/oxyn/relation-structure"
import { ResultPanel } from "@/components/oxyn/result-panel"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import {
  previewUnavailable,
  unsupportedReason,
} from "@/features/metadata/capabilities"
import { copyToClipboard } from "@/features/metadata/clipboard"
import { inspectObject, inspection } from "@/features/metadata/inspection"
import { usePreview } from "@/features/metadata/use-preview"
import { useRelationFacets } from "@/features/metadata/use-relation-facets"
import {
  ValueInspectionDialog,
  gridInspection,
  useReleaseSelection,
} from "@/features/metadata/value-inspection"
import { ExportMenu } from "@/features/workspace/export-menu"
import { metadata } from "@/lib/ipc/metadata"
import type { BoundValue } from "@/lib/ipc/metadata"
import { results } from "@/lib/ipc/results"
import type {
  CatalogAddress,
  CatalogNode,
  OpenConnection,
} from "@/lib/ipc/types"

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
  initialTab,
  onOpenRelated,
  onOpenInConsole,
}: {
  open: OpenConnection
  node: CatalogNode
  initialTab?: OpenTarget
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
    parameters?: Array<BoundValue>,
    /** A value is still missing: the console says so before running. */
    needsValues?: boolean
  ) => void
}) {
  const dataUnavailable = previewUnavailable(open, node.holdsRecords)
  const previewable = dataUnavailable === null
  const [tab, setTab] = React.useState<ObjectTab>(() =>
    initialObjectTab(initialTab, previewable)
  )
  const [direction, setDirection] = React.useState<"outgoing" | "incoming">(
    () => (unsupportedReason(open, "incoming") ? "outgoing" : "incoming")
  )
  const source = `preview:${open.connection}:${addressKey(node.address)}`
  const facets = useRelationFacets(open, node.address)
  const preview = usePreview({
    open,
    address: node.address,
    enabled: previewable,
    visible: tab === "data",
  })

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
  }, [tab, direction, ensure])

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
          onOpenInConsole(
            query.sql,
            "Related rows.sql",
            query.parameters,
            query.needsValues
          )
      }
    : undefined

  const detailLoad = facets.loads.detail

  return (
    <ObjectViewFrame
      name={node.name}
      kind={data?.kind ?? node.kind}
      tab={tab}
      onTabChange={setTab}
      dataUnavailable={dataUnavailable}
      onEscape={tab === "data" && preview.running ? preview.cancel : undefined}
      toolbar={
        tab === "data" && previewable ? (
          <PreviewToolbar
            running={preview.running}
            cancelling={preview.cancelling}
            onRefresh={preview.refresh}
            onCancel={preview.cancel}
          />
        ) : null
      }
      notice={
        facets.error ? (
          <Alert
            variant="destructive"
            className="rounded-none border-x-0 border-t-0"
          >
            <AlertTitle>The catalog cache could not be read</AlertTitle>
            <AlertDescription className="font-mono text-xs text-foreground">
              {facets.error}
            </AlertDescription>
          </Alert>
        ) : null
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
                footerActions={
                  <ExportMenu
                    connection={open.connection}
                    result={result}
                    exportable={exportable}
                    reason={
                      state.status === "populated"
                        ? "This preview is partial: it was cancelled or truncated."
                        : "Export becomes available once the preview has finished loading."
                    }
                    label="Export preview…"
                    scope={
                      state.status === "populated"
                        ? `The ${state.rows.toLocaleString("en-US")} preview rows shown · not the entire table`
                        : "The preview rows shown · not the entire table"
                    }
                    defaultName={node.name}
                  />
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
            <RelationStructure detail={data.detail.value} />
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
            onDirectionChange={setDirection}
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
                  onCopy={(sql) => void copyToClipboard(sql, "DDL")}
                  onOpenInConsole={
                    onOpenInConsole
                      ? (sql) => onOpenInConsole(sql, "Object DDL")
                      : undefined
                  }
                />
              ) : null
            }
          >
            {data.definition.value ? (
              <RelationDefinition
                definition={data.definition.value}
                stale={
                  data.definition.freshness.state === "invalidated" ||
                  facets.loads.definition.status === "error"
                }
              />
            ) : null}
          </FacetFrame>
        ) : null,
      }}
    />
  )
}
