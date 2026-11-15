import * as React from "react"
import { useQuery, useQueryClient } from "@tanstack/react-query"
import { SchemaMapView } from "@/components/oxyn/schema-map-view"
import type { SchemaMapViewState } from "@/components/oxyn/schema-map-view"
import { SchemaSamplingDialog } from "@/components/oxyn/schema-sampling-dialog"
import type { OpenTarget } from "@/components/oxyn/catalog-tree"
import { mapReference, mapScopes, scopeKey } from "./scopes"
import { BackendError, backend, newCommandId } from "@/lib/ipc/client"
import { schemaMaps } from "@/lib/ipc/schema-map"
import type { MapObject, MapObjectRef } from "@/lib/ipc/schema-map"
import { metadata } from "@/lib/ipc/metadata"
import type { MetadataScope } from "@/lib/ipc/metadata-loading"
import type { CatalogNode, OpenConnection } from "@/lib/ipc/types"
import { useRefreshSignal } from "@/features/metadata/refresh-signals"
import { hasCapability } from "@/features/session"

/** Opening/selecting a scope demands its bounded map; visual gestures stay local. */
export function SchemaMapTab({
  open,
  active,
  initialScope,
  initialObject,
  onOpenObject,
}: {
  open: OpenConnection
  active: boolean
  initialScope: MetadataScope | null
  initialObject?: CatalogNode
  onOpenObject: (
    node: CatalogNode,
    target: OpenTarget,
    owner: OpenConnection
  ) => void
}) {
  const client = useQueryClient()
  const [scope, setScope] = React.useState<MetadataScope | null>(initialScope)
  const seeds = React.useRef<Array<MapObjectRef>>(
    initialObject
      ? [mapReference(initialObject.address)].filter(
          (value): value is MapObjectRef => value !== null
        )
      : []
  )
  const [selected, setSelected] = React.useState<string | null>(null)
  const [review, setReview] = React.useState<MapObject | null>(null)
  const [state, setState] = React.useState<SchemaMapViewState>({
    data: null,
    loading: false,
    error: null,
  })
  const operation = React.useRef<string | null>(null)
  const attempted = React.useRef<string | null>(null)
  const live = React.useRef(true)
  const shown = React.useRef(active)
  shown.current = active
  const tree = useQuery({
    queryKey: ["catalog", open.connection, open.session],
    queryFn: () => metadata.catalogTree(open.connection, open.session),
  })
  const scopes = mapScopes(tree.data ?? [], hasCapability(open, "SCHEMAS"))
  if (scope && !scopes.some((choice) => choice.key === scopeKey(scope)))
    scopes.push({
      key: scopeKey(scope),
      scope,
      label:
        [scope.catalog, scope.namespace]
          .filter((part) => part !== null)
          .join(" / ") || "Source root",
    })
  React.useEffect(() => {
    live.current = true
    return () => {
      live.current = false
      const id = operation.current
      operation.current = null
      if (id) void backend.cancel(id).catch(() => undefined) // Backend/window ownership still awaits cleanup.
    }
  }, [])
  React.useEffect(() => {
    if (active) return
    setReview(null)
    const id = operation.current
    if (id) {
      operation.current = null
      attempted.current = null
      void backend.cancel(id).catch(() => undefined)
      setState((previous) => ({
        ...previous,
        loading: false,
        cancelling: false,
      }))
    }
  }, [active])
  const generation = () =>
    client.fetchQuery({
      queryKey: ["catalog-generation", open.connection, open.session],
      queryFn: () => metadata.catalogGeneration(open.connection, open.session),
      staleTime: 0,
    })
  useRefreshSignal(open.connection, (signal) => {
    if (signal.type === "catalogInvalidated" || signal.type === "lagged") {
      setState((previous) => ({ ...previous, stale: previous.data !== null }))
    }
    if (
      (signal.type === "catalogChanged" ||
        signal.type === "catalogInvalidated" ||
        signal.type === "lagged") &&
      state.data
    ) {
      void generation()
        .then((epoch) => {
          if (live.current)
            setState((previous) =>
              previous.data &&
              previous.data.projection.source.generation !== epoch
                ? { ...previous, stale: true }
                : previous
            )
        })
        .catch((caught: unknown) => {
          if (live.current)
            setState((previous) => ({
              ...previous,
              stale: previous.data !== null,
              error: failure(caught),
            }))
        })
    }
  })
  const current = (id: string) => live.current && operation.current === id
  const failure = (caught: unknown) => ({
    message: caught instanceof Error ? caught.message : String(caught),
    retryable: caught instanceof BackendError && caught.retryable,
  })
  const load = async (
    refresh: boolean,
    objects = seeds.current,
    expand: MapObjectRef | null = null
  ) => {
    if (operation.current) return
    const id = newCommandId()
    operation.current = id
    setState((previous) => ({
      ...previous,
      loading: true,
      cancelling: false,
      error: null,
      stale: refresh && previous.data !== null,
    }))
    try {
      const data = await schemaMaps.load(id, {
        connection: open.connection,
        session: open.session,
        scope: scope ?? { catalog: null, namespace: null },
        objects,
        expand,
        refresh,
      })
      if (current(id)) {
        if (
          data.projection.source.connection !== open.connection ||
          data.projection.source.session !== open.session
        )
          throw new Error("The map answered for another owner.")
        setState({
          data,
          loading: false,
          error: null,
          stale:
            data.stopped !== null ||
            data.issues.some((issue) => issue.kind === "failed"),
        })
        setSelected((key) =>
          data.projection.objects.some((object) => object.key === key)
            ? key
            : (data.projection.objects[0]?.key ?? null)
        )
      }
    } catch (caught) {
      if (current(id))
        setState((previous) => ({
          ...previous,
          loading: false,
          error: failure(caught),
          stale: previous.data !== null,
        }))
    } finally {
      await client.invalidateQueries({
        queryKey: ["catalog", open.connection, open.session],
      })
      if (current(id)) {
        operation.current = null
        setState((previous) => ({
          ...previous,
          loading: false,
          cancelling: false,
        }))
      }
    }
  }
  const demandKey = JSON.stringify(scope ?? { catalog: null, namespace: null })
  const demand = React.useRef(load)
  demand.current = load
  React.useEffect(() => {
    if (!active || attempted.current === demandKey) return
    let abandoned = false
    // StrictMode's discarded effect must not issue a second native demand.
    void Promise.resolve().then(() => {
      if (abandoned || !live.current || !shown.current) return
      attempted.current = demandKey
      void demand.current(false)
    })
    return () => {
      abandoned = true
    }
  }, [active, demandKey])
  React.useEffect(() => {
    if (!active || seeds.current.length || operation.current) return
    if (scope === null && state.data && !state.data.chooseNamespace) {
      setScope({ catalog: null, namespace: null })
      return
    }
    const children = scopes.filter(
      (choice) =>
        choice.scope.namespace !== null &&
        (scope?.catalog == null || choice.scope.catalog === scope.catalog)
    )
    const oneCatalog =
      new Set(scopes.map((choice) => choice.scope.catalog)).size <= 1
    const choice =
      scope === null &&
      scopes.length === 1 &&
      (!state.data || state.data.chooseNamespace)
        ? scopes[0]
        : state.data?.chooseNamespace &&
            scope?.namespace == null &&
            children.length === 1 &&
            (scope?.catalog != null || oneCatalog)
          ? children[0]
          : undefined
    if (
      choice &&
      scopeKey(choice.scope) !==
        scopeKey(scope ?? { catalog: null, namespace: null })
    ) {
      setScope(choice.scope)
      setState({ data: null, loading: false, error: null })
    } else if (choice && scope === null) setScope(choice.scope)
  }, [active, tree.data, state.data, state.loading, scope])
  const cancel = async () => {
    const id = operation.current
    if (!id) return
    setState((previous) => ({ ...previous, cancelling: true }))
    try {
      await backend.cancel(id)
    } catch (caught) {
      if (current(id))
        setState((previous) => ({
          ...previous,
          cancelling: false,
          error: failure(caught),
        }))
    }
  }
  const retained = () =>
    state.data?.projection.objects
      .map((object) => mapReference(object.address))
      .filter((value): value is MapObjectRef => value !== null) ?? seeds.current
  const infer = async (object: MapObject) => {
    if (operation.current) return
    setReview(null)
    const id = newCommandId()
    operation.current = id
    setState((previous) => ({ ...previous, loading: true, error: null }))
    let loaded = false
    try {
      const answer = await metadata.refreshRelationFacet(
        id,
        open.connection,
        open.session,
        object.address,
        "sampledDescription"
      )
      loaded = answer.type === "catalogRefreshed"
      if (answer.type === "needsApproval") {
        await backend.decide(answer.command, false)
        throw new Error(
          "The inference read required an unexpected approval and was withdrawn."
        )
      }
      if (answer.type === "denied") throw new Error(answer.reason)
    } catch (caught) {
      if (current(id))
        setState((previous) => ({ ...previous, error: failure(caught) }))
    } finally {
      if (current(id)) {
        operation.current = null
        setState((previous) => ({ ...previous, loading: false }))
      }
    }
    if (loaded && live.current && shown.current) await load(false, retained())
  }
  const openObject = (object: MapObject, target: OpenTarget) =>
    onOpenObject(
      {
        address: object.address,
        name: object.address.relation ?? "",
        kind: object.kind,
        holdsRecords: object.holdsRecords,
        system: false,
        comment: null,
        loaded: object.fieldsState !== "unread",
        stale: object.fieldsState === "stale",
        children: [],
        virtualTable: null,
        shadowOf: null,
      },
      target,
      open
    )
  return (
    <>
      <SchemaMapView
        connection={open.name}
        scopes={scopes}
        scopeKey={scope ? scopeKey(scope) : null}
        state={state}
        selected={selected}
        definitionSupported={hasCapability(open, "OBJECT_DEFINITION")}
        onScope={(key) => {
          const choice = scopes.find((candidate) => candidate.key === key)
          if (
            choice &&
            scopeKey(choice.scope) !==
              scopeKey(scope ?? { catalog: null, namespace: null })
          ) {
            const previous = operation.current
            operation.current = null
            if (previous) void backend.cancel(previous).catch(() => undefined)
            setScope(choice.scope)
            seeds.current = []
            setSelected(null)
            setState({ data: null, loading: false, error: null })
          }
        }}
        selectionLabel={
          seeds.current.length
            ? seeds.current.map((object) => object.relation).join(", ")
            : undefined
        }
        onLoadScope={() => {
          seeds.current = []
          void load(false, [])
        }}
        onLoad={(refresh) =>
          void load(refresh, refresh ? retained() : seeds.current)
        }
        onCancel={() => void cancel()}
        onSelect={setSelected}
        onOpen={openObject}
        onExpand={(object) =>
          void load(false, retained(), mapReference(object.address))
        }
        onInfer={(object) => setReview(object)}
      />
      {active && review && (
        <SchemaSamplingDialog
          connection={open.name}
          object={review.address.relation ?? "Object"}
          limits={state.data?.projection.sampleLimits ?? null}
          onSample={() => void infer(review)}
          onCancel={() => setReview(null)}
        />
      )}
    </>
  )
}
