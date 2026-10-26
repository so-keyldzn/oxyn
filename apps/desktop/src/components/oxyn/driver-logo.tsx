import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import { DatabaseIcon } from "@hugeicons/core-free-icons"

import { cn } from "@/lib/utils"

import { Bigquery } from "@/components/ui/svgs/bigquery"
import { Cassandra } from "@/components/ui/svgs/cassandra"
import { Chroma } from "@/components/ui/svgs/chroma"
import { ChromaDarkMode } from "@/components/ui/svgs/chromaDarkMode"
import { Clickhouse } from "@/components/ui/svgs/clickhouse"
import { Couchbase } from "@/components/ui/svgs/couchbase"
import { Duckdb } from "@/components/ui/svgs/duckdb"
import { DuckdbDarkMode } from "@/components/ui/svgs/duckdbDarkMode"
import { Dynamodb } from "@/components/ui/svgs/dynamodb"
import { Elasticsearch } from "@/components/ui/svgs/elasticsearch"
import { Memgraph } from "@/components/ui/svgs/memgraph"
import { Milvus } from "@/components/ui/svgs/milvus"
import { MilvusDarkMode } from "@/components/ui/svgs/milvusDarkMode"
import { MongodbIconLight } from "@/components/ui/svgs/mongodbIconLight"
import { MysqlIconLight } from "@/components/ui/svgs/mysqlIconLight"
import { Neo4j } from "@/components/ui/svgs/neo4j"
import { Neo4jDark } from "@/components/ui/svgs/neo4jDark"
import { Opensearch } from "@/components/ui/svgs/opensearch"
import { OpensearchDarkMode } from "@/components/ui/svgs/opensearchDarkMode"
import { Pinecone } from "@/components/ui/svgs/pinecone"
import { PineconeDarkMode } from "@/components/ui/svgs/pineconeDarkMode"
import { Postgresql } from "@/components/ui/svgs/postgresql"
import { QdrantIconDark } from "@/components/ui/svgs/qdrantIconDark"
import { QdrantIconLight } from "@/components/ui/svgs/qdrantIconLight"
import { Redis } from "@/components/ui/svgs/redis"
import { Snowflake } from "@/components/ui/svgs/snowflake"
import { SqlServer } from "@/components/ui/svgs/sqlServer"
import { Sqlite } from "@/components/ui/svgs/sqlite"
import { WeaviateDarkMode } from "@/components/ui/svgs/weaviateDarkMode"

type Logo = (props: React.SVGProps<SVGSVGElement>) => React.ReactElement

/** A generated mark and the size of its own viewBox, in its user units. */
type Art = { Mark: Logo; width: number; height: number }

/**
 * Draws generated marks inside a frame of our own. The frame's `viewBox`
 * crops a lockup down to its symbol; a second variant lets the theme pick the
 * one the vendor drew for a dark background, rather than recoloring either.
 * A frame that is not square sits centred in the square box, at its aspect
 * ratio. The generated files stay untouched, so a refresh cannot undo this.
 */
function framed(viewBox: string, light: Art, dark?: Art): Logo {
  function FramedMark(props: React.SVGProps<SVGSVGElement>) {
    return (
      <svg viewBox={viewBox} {...props}>
        <light.Mark
          width={light.width}
          height={light.height}
          className={dark && "dark:hidden"}
        />
        {dark && (
          <dark.Mark
            width={dark.width}
            height={dark.height}
            className="hidden dark:inline"
          />
        )}
      </svg>
    )
  }
  return FramedMark
}

/**
 * svgl only has SQLite's horizontal lockup (512 × 228), whose wordmark is
 * navy on a dark theme and shrinks the mark to nothing in a square. This
 * frames the square and feather, clipped along the gap before the « S »;
 * the generated file stays untouched, so `shadcn add` can refresh it.
 */
function SqliteMark(props: React.SVGProps<SVGSVGElement>) {
  const clip = React.useId()
  return (
    <svg viewBox="0 0 206 228" {...props}>
      <defs>
        <clipPath id={clip}>
          <polygon points="0,0 206,0 206,100 171,109 166,228 0,228" />
        </clipPath>
      </defs>
      <g clipPath={`url(#${clip})`}>
        <Sqlite width={512} height={228} />
      </g>
    </svg>
  )
}

/** The token overrides svgl's fixed fill so one mark follows both themes. */
function MysqlMark({ className, ...props }: React.SVGProps<SVGSVGElement>) {
  return (
    <MysqlIconLight
      {...props}
      className={cn(className, "[&_path]:fill-(--driver-mysql)")}
    />
  )
}

/**
 * DuckDB publishes its mark only as stacked lockups, one per background:
 * a black disc for light surfaces, a yellow one for dark. This frames the
 * disc of each — the wordmark falls outside the square — and lets the theme
 * pick the variant the vendor drew for it, rather than recoloring either.
 */
function DuckdbMark({ className, ...props }: React.SVGProps<SVGSVGElement>) {
  return (
    <svg viewBox="0 0 334.4 334.4" className={className} {...props}>
      <Duckdb
        x={-217.6}
        y={-10}
        width={769.5}
        height={591.9}
        className="dark:hidden"
      />
      <DuckdbDarkMode
        x={-234.2}
        y={-12.5}
        width={802.9}
        height={609.4}
        className="hidden dark:inline"
      />
    </svg>
  )
}

/**
 * EUI leaves the middle bar unfilled and paints it with the text color. The
 * selector avoids the class's `__`: Tailwind reads `_` as a space in a variant.
 */
function ElasticsearchMark({
  className,
  ...props
}: React.SVGProps<SVGSVGElement>) {
  return (
    <Elasticsearch
      {...props}
      className={cn(className, "[&_[class$=fillNegative]]:fill-foreground")}
    />
  )
}

/**
 * The leaf as svgl draws it on a light surface; the token takes svgl's dark
 * variant's green on a dark one.
 */
function MongodbMark({ className, ...props }: React.SVGProps<SVGSVGElement>) {
  return (
    <MongodbIconLight
      {...props}
      className={cn(className, "[&_path]:fill-(--driver-mongodb)")}
    />
  )
}

/** Chroma's dark lockup, cropped to its two discs. */
const ChromaDarkSymbol = framed("0 0 3.94 2.565", {
  Mark: ChromaDarkMode,
  width: 12,
  height: 3,
})

/**
 * The official marks, keyed by driver id. They come from the svgl registry
 * (`shadcn add @svgl/<name>`) or, where svgl has none, from the vendors' own
 * files, paths unmodified, with their source in each header.
 *
 * Oracle has none on purpose: its brand requires written authorization and
 * publishes no square mark, so it keeps the generic icon.
 *
 * A driver per protocol (ADR-0003): Redshift reaches Oxyn through `postgres`
 * and wears its mark — but OpenSearch, which the `elasticsearch` driver also
 * reaches, has a different mark of its own: keyed by driver id, an OpenSearch
 * connection wears Elasticsearch's, and only the catalogue's `opensearch`
 * entry wears OpenSearch's.
 */
const logos: Partial<Record<string, Logo>> = {
  postgres: Postgresql,
  mysql: MysqlMark,
  mongodb: MongodbMark,
  sqlite: SqliteMark,
  sqlserver: SqlServer,
  duckdb: DuckdbMark,
  clickhouse: Clickhouse,
  // The guidelines require the ® with the bug: it is not cropped off.
  snowflake: Snowflake,
  bigquery: Bigquery,
  redis: Redis,
  // The lockup's wordmark is white, invisible on a light page: only the eye.
  cassandra: framed("0 20 142 142.5", {
    Mark: Cassandra,
    width: 517.5,
    height: 182.51,
  }),
  dynamodb: Dynamodb,
  couchbase: Couchbase,
  milvus: framed(
    "0 0 360 360",
    { Mark: Milvus, width: 360, height: 360 },
    { Mark: MilvusDarkMode, width: 360, height: 360 }
  ),
  // Weaviate publishes its 2026 lockup for dark backgrounds only; the symbol
  // is worn on both themes, never recolored into a light variant of our own.
  weaviate: framed("0 0 30 18", {
    Mark: WeaviateDarkMode,
    width: 122,
    height: 18,
  }),
  pinecone: framed(
    "0 0 220 220",
    { Mark: Pinecone, width: 1077, height: 220 },
    { Mark: PineconeDarkMode, width: 1077, height: 220 }
  ),
  qdrant: framed(
    "0 0 57 64",
    { Mark: QdrantIconLight, width: 57, height: 64 },
    { Mark: QdrantIconDark, width: 57, height: 64 }
  ),
  chromadb: framed(
    "0 0 209 135",
    { Mark: Chroma, width: 209, height: 135 },
    { Mark: ChromaDarkSymbol, width: 209, height: 135 }
  ),
  neo4j: framed(
    "0 0 86 79",
    { Mark: Neo4j, width: 86, height: 79 },
    { Mark: Neo4jDark, width: 86, height: 79 }
  ),
  memgraph: Memgraph,
  elasticsearch: ElasticsearchMark,
  opensearch: framed(
    "0 0 64 64",
    { Mark: Opensearch, width: 64, height: 64 },
    { Mark: OpensearchDarkMode, width: 64, height: 64 }
  ),
}

/**
 * The driver's official logo, decorative: the driver's name is always written
 * next to it. A driver without a known mark gets the generic database icon,
 * so a new driver is never drawn without one.
 */
export function DriverLogo({
  driver,
  className,
}: {
  /** The driver id, as `DriverChoice.id` and `ConnectionSummary.driver`. */
  driver: string
  className?: string
}) {
  const Mark = logos[driver]
  if (Mark) return <Mark aria-hidden className={className} />
  return (
    <HugeiconsIcon
      icon={DatabaseIcon}
      strokeWidth={2}
      aria-hidden
      className={className}
    />
  )
}
