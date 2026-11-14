import type { MapObject, SchemaMapLoaded } from "@/lib/ipc/schema-map"

/** Synthetic metadata only: no account, host, row value or real source identity. */
export function mapFixture(kind = "table"): SchemaMapLoaded {
  const object = (namespace: string, name: string): MapObject => ({
    key: JSON.stringify(["fixture-owner", namespace, name]),
    address: { catalog: null, namespace, relation: name },
    kind,
    holdsRecords: true,
    fields: [
      {
        key: `${namespace}:${name}:id`,
        parent: null,
        path: ["id"],
        name: "id",
        rawType: "integer",
        typeShortened: false,
        nullable: false,
        primaryKey: true,
        origin: "reported",
      },
    ],
    omittedFields: 0,
    fieldsState: "fresh",
    samplingRequired: false,
    keysState: "fresh",
    keysSupported: true,
  })
  const orders = object("sales", "orders")
  const archive = object("archive", "orders")
  return {
    projection: {
      source: {
        connection: "fixture-source",
        session: "fixture-owner",
        generation: "00000000-0000-4000-8000-000000000001",
      },
      containers: [],
      objects: [orders, archive],
      links: [
        {
          key: "composite-key",
          kind: "foreignKey",
          from: orders.key,
          to: archive.key,
          target: archive.address,
          targetLoaded: true,
          name: "reported composite",
          fields: ["tenant", "parent_id"],
          referencedFields: ["tenant", "id"],
          wellFormed: true,
          freshness: "fresh",
          sourceUnique: null,
          targetUnique: null,
        },
      ],
      coverage: {
        knownObjects: 2,
        drawnObjects: 2,
        omittedObjects: 0,
        omittedFields: 0,
        omittedLinks: 0,
        unreadObjects: 0,
        staleObjects: 0,
        unresolvedLinks: 0,
      },
      dependenciesSupported: false,
      sampleLimits: null,
    },
    reads: 5,
    issues: [],
    chooseNamespace: false,
    stopped: null,
  }
}
