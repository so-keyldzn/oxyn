import { SchemaMapLinkList } from "./schema-map-link-list"
import * as React from "react"
import { SchemaMapInspector } from "./schema-map-inspector"
import { erdProjection } from "./schema-map-model"
import { BackendErrorAlert } from "./backend-error-alert"
import type { BackendFailure } from "./backend-error-alert"
import type { OpenTarget } from "./catalog-tree"
import { Button } from "@/components/ui/button"
import { Badge } from "@/components/ui/badge"
import { Field, FieldGroup, FieldTitle } from "@/components/ui/field"
import {
  Select,
  SelectTrigger,
  SelectValue,
  SelectContent,
  SelectGroup,
  SelectItem,
} from "@/components/ui/select"
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs"
import {
  Empty,
  EmptyHeader,
  EmptyTitle,
  EmptyDescription,
} from "@/components/ui/empty"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Spinner } from "@/components/ui/spinner"
import {
  Collapsible,
  CollapsibleTrigger,
  CollapsibleContent,
} from "@/components/ui/collapsible"
import { TextInput } from "./text-field"
import type { MetadataScope } from "@/lib/ipc/metadata-loading"
import type { MapObject, SchemaMapLoaded } from "@/lib/ipc/schema-map"

const Diagram = React.lazy(() =>
  import("./erd-diagram").then((module) => ({ default: module.ErdDiagram }))
)
export interface MapScopeChoice {
  key: string
  label: string
  scope: MetadataScope
}
export interface SchemaMapViewState {
  data: SchemaMapLoaded | null
  loading: boolean
  cancelling?: boolean
  error: BackendFailure | null
  stale?: boolean
}

/** Source-neutral map surface; selection, filtering and canvas gestures do no I/O. */
export function SchemaMapView({
  connection,
  scopes,
  scopeKey,
  state,
  selected,
  definitionSupported,
  onScope,
  onLoad,
  selectionLabel,
  onLoadScope,
  onCancel,
  onSelect,
  onOpen,
  onExpand,
  onInfer,
}: {
  connection: string
  scopes: Array<MapScopeChoice>
  scopeKey: string | null
  state: SchemaMapViewState
  selected: string | null
  definitionSupported: boolean
  onScope: (key: string) => void
  onLoad: (refresh: boolean) => void
  selectionLabel?: string
  onLoadScope?: () => void
  onCancel: () => void
  onSelect: (key: string) => void
  onOpen: (object: MapObject, target: OpenTarget) => void
  onExpand: (object: MapObject) => void
  onInfer?: (object: MapObject) => void
}) {
  const scopeLabel = React.useId()
  const [filter, setFilter] = React.useState("")
  const data = state.data
  const projection = data?.projection
  const objects = projection?.objects ?? []
  const filtered = objects.filter((object) =>
    [
      object.address.catalog,
      object.address.namespace,
      object.address.relation,
      object.kind,
    ]
      .filter((part) => part !== null)
      .join(" ")
      .toLowerCase()
      .includes(filter.toLowerCase())
  )
  const visibleKeys = React.useMemo(
    () => new Set(filtered.map((object) => object.key)),
    [filtered]
  )
  const graph = React.useMemo(
    () => (projection ? erdProjection(projection) : null),
    [projection]
  )
  const chosen = objects.find((object) => object.key === selected)
  return (
    <section
      aria-label="Schema map"
      data-focus-region="schema-map"
      data-focus-entry="true"
      tabIndex={-1}
      className="flex h-full min-h-0 min-w-0 flex-col gap-3 p-4"
    >
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h2 className="text-base font-semibold">Schema map</h2>
          <p className="text-sm text-muted-foreground">
            <bdi>{connection}</bdi> · Local metadata, no AI request
          </p>
        </div>
        <div className="flex flex-wrap gap-2">
          {state.loading ? (
            <Button
              variant="outline"
              disabled={state.cancelling}
              onClick={onCancel}
            >
              <Spinner aria-hidden data-icon="inline-start" />
              {state.cancelling ? "Cancelling…" : "Cancel read"}
            </Button>
          ) : (
            <Button variant="outline" onClick={() => onLoad(true)}>
              Refresh map
            </Button>
          )}
        </div>
      </div>
      <FieldGroup className="flex-row flex-wrap items-end gap-3">
        <Field className="min-w-48 flex-1" data-disabled={state.loading}>
          <FieldTitle id={scopeLabel}>Native scope</FieldTitle>
          <Select
            items={scopes.map((scope) => ({
              value: scope.key,
              label: scope.label,
            }))}
            value={scopeKey}
            disabled={state.loading}
            onValueChange={(key) => {
              if (key !== null) onScope(key)
            }}
          >
            <SelectTrigger aria-labelledby={scopeLabel} className="w-full">
              <SelectValue placeholder="Read available scopes first" />
            </SelectTrigger>
            <SelectContent>
              <SelectGroup>
                {scopes.map((scope) => (
                  <SelectItem key={scope.key} value={scope.key}>
                    {scope.label}
                  </SelectItem>
                ))}
              </SelectGroup>
            </SelectContent>
          </Select>
        </Field>
        <Button disabled={state.loading} onClick={() => onLoad(false)}>
          {selectionLabel
            ? `Load ${selectionLabel}`
            : scopeKey === null
              ? "Read available scopes"
              : "Load selected scope"}
        </Button>
        {selectionLabel && onLoadScope && (
          <Button
            variant="outline"
            disabled={state.loading}
            onClick={onLoadScope}
          >
            Load whole selected scope
          </Button>
        )}
        <Field className="min-w-48 flex-1">
          <FieldTitle>Filter loaded objects</FieldTitle>
          <TextInput
            aria-label="Filter loaded objects"
            placeholder="Name, namespace or kind…"
            value={filter}
            onChange={(event) => setFilter(event.target.value)}
          />
        </Field>
      </FieldGroup>
      {state.error && (
        <BackendErrorAlert
          title="Map read did not complete"
          error={state.error}
          nextStep="Check the original source/session, or choose a smaller scope. The previous map is retained."
        />
      )}
      {state.loading && (
        <p role="status" className="text-sm text-muted-foreground">
          Reading bounded metadata on the original session. Documentary samples
          are excluded.
        </p>
      )}
      {(state.stale ||
        (projection && projection.coverage.staleObjects > 0)) && (
        <p role="status" className="text-sm text-muted-foreground">
          Retained metadata may be stale. Refresh or open Structure for an
          explicit read.
        </p>
      )}
      {projection && (
        <div className="flex flex-wrap gap-2 text-xs" aria-label="Map coverage">
          <Badge variant="secondary">
            {projection.coverage.drawnObjects} of{" "}
            {projection.coverage.knownObjects} known objects
          </Badge>
          <Badge variant="outline">
            {projection.coverage.omittedObjects} objects omitted
          </Badge>
          <Badge variant="outline">
            {projection.coverage.omittedFields} fields omitted
          </Badge>
          <Badge variant="outline">
            {projection.coverage.omittedLinks} links omitted
          </Badge>
          <Badge variant="outline">
            {projection.coverage.unreadObjects} unread descriptions
          </Badge>
          <Badge variant="outline">
            {projection.coverage.unresolvedLinks} unloaded endpoints
          </Badge>
        </div>
      )}
      {data && (data.issues.length > 0 || data.stopped) && (
        <Collapsible className="text-xs text-muted-foreground">
          <CollapsibleTrigger render={<Button variant="ghost" size="sm" />}>
            {data.issues.length} partial/unavailable metadata parts
            {data.stopped ? ` · read stopped: ${data.stopped}` : ""}
          </CollapsibleTrigger>
          <CollapsibleContent>
            <ul className="list-inside list-disc">
              {data.issues.map((issue, index) => (
                <li key={index}>
                  {[
                    issue.address.catalog,
                    issue.address.namespace,
                    issue.address.relation,
                  ]
                    .filter((part) => part !== null)
                    .join(".") || "Source"}
                  : {issue.facet} · {issue.kind}
                </li>
              ))}
            </ul>
          </CollapsibleContent>
        </Collapsible>
      )}
      {!data || data.chooseNamespace || objects.length === 0 ? (
        <Empty className="min-h-48 flex-1">
          <EmptyHeader>
            <EmptyTitle>
              {state.loading
                ? "Reading metadata"
                : data?.stopped
                  ? "Map read stopped"
                  : data?.chooseNamespace
                    ? "Choose a native child scope"
                    : data?.issues.some((issue) => issue.facet === "names")
                      ? "No observed object to draw"
                      : data
                        ? "No objects reported in this scope"
                        : "Choose a scope to draw its metadata"}
            </EmptyTitle>
            <EmptyDescription>
              {data?.chooseNamespace
                ? "Only the selected container level was read. Select its schema/database and load it explicitly."
                : "Objects remain useful without foreign keys. No rows are read by drawing, panning, filtering or selecting."}
            </EmptyDescription>
          </EmptyHeader>
        </Empty>
      ) : (
        <div
          data-slot="schema-map-content"
          className="grid min-h-0 flex-1 auto-rows-max gap-3 overflow-auto lg:auto-rows-auto lg:grid-cols-[minmax(0,1fr)_20rem]"
        >
          <Tabs
            defaultValue="diagram"
            className="flex min-h-96 min-w-0 flex-col lg:min-h-0"
          >
            <TabsList aria-label="Map representation">
              <TabsTrigger value="diagram">Diagram</TabsTrigger>
              <TabsTrigger value="list">Accessible list</TabsTrigger>
            </TabsList>
            <TabsContent value="diagram" className="min-h-0 flex-1">
              <React.Suspense
                fallback={<p role="status">Loading diagram renderer…</p>}
              >
                {graph && (
                  <Diagram
                    tables={graph.tables}
                    links={graph.links}
                    visibleKeys={visibleKeys}
                    actionLabel="Inspect"
                    omitted={projection?.coverage.omittedObjects ?? 0}
                    onOpenObject={(address) => {
                      const object = objects.find(
                        (candidate) =>
                          JSON.stringify(candidate.address) ===
                          JSON.stringify(address)
                      )
                      if (object) onSelect(object.key)
                    }}
                    className="h-full min-h-80"
                  />
                )}
              </React.Suspense>
            </TabsContent>
            <TabsContent value="list" className="min-h-0 flex-1">
              <ScrollArea className="h-full">
                <ul aria-label="Mapped objects" className="flex flex-col gap-2">
                  {filtered.map((object) => (
                    <li
                      key={object.key}
                      className="flex flex-wrap items-center gap-2 rounded-md border p-2"
                    >
                      <Button
                        variant="ghost"
                        onClick={() => onSelect(object.key)}
                      >
                        Inspect {object.address.relation}
                      </Button>
                      <Badge variant="outline">{object.kind}</Badge>
                      <span className="text-xs text-muted-foreground">
                        {[object.address.catalog, object.address.namespace]
                          .filter((part) => part !== null)
                          .join(" / ") || "Source root"}{" "}
                        · fields {object.fieldsState} · keys{" "}
                        {object.keysSupported
                          ? object.keysState
                          : "unavailable"}
                      </span>
                    </li>
                  ))}
                </ul>
                {projection && <SchemaMapLinkList projection={projection} />}
                {filtered.length === 0 && (
                  <p className="p-3 text-sm">
                    No loaded object matches this filter.
                  </p>
                )}
              </ScrollArea>
            </TabsContent>
          </Tabs>
          {chosen && projection ? (
            <SchemaMapInspector
              object={chosen}
              projection={projection}
              busy={state.loading}
              definitionSupported={definitionSupported}
              onOpen={onOpen}
              onExpand={onExpand}
              onInfer={onInfer}
            />
          ) : (
            <Empty className="hidden lg:flex">
              <EmptyHeader>
                <EmptyTitle>Select an object</EmptyTitle>
                <EmptyDescription>
                  Inspect reported structure and explicitly expand neighbours or
                  open another view.
                </EmptyDescription>
              </EmptyHeader>
            </Empty>
          )}
        </div>
      )}
      <p className="text-xs text-muted-foreground">
        Foreign keys are reported constraints, not name-based guesses. Native
        membership and field nesting have separate meanings. Incoming links
        cover loaded objects only.{" "}
        {projection?.dependenciesSupported
          ? ""
          : "Structured technical dependencies are unavailable from the current provider contract."}{" "}
        No complete database schema is claimed.
      </p>
    </section>
  )
}
