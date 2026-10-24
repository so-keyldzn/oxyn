import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import { DatabaseIcon } from "@hugeicons/core-free-icons"

import { cn } from "@/lib/utils"

import { Clickhouse } from "@/components/ui/svgs/clickhouse"
import { Duckdb } from "@/components/ui/svgs/duckdb"
import { DuckdbDarkMode } from "@/components/ui/svgs/duckdbDarkMode"
import { Elasticsearch } from "@/components/ui/svgs/elasticsearch"
import { MysqlIconLight } from "@/components/ui/svgs/mysqlIconLight"
import { Postgresql } from "@/components/ui/svgs/postgresql"
import { Sqlite } from "@/components/ui/svgs/sqlite"

type Logo = (props: React.SVGProps<SVGSVGElement>) => React.ReactElement

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
 * The official marks, keyed by driver id. Most come from the svgl registry
 * (`shadcn add @svgl/<name>`); svgl has none for ClickHouse, DuckDB or
 * Elasticsearch, so theirs are the vendors' own files, paths unmodified, with
 * their source in each header. A driver per protocol (ADR-0003): Redshift
 * reaches Oxyn through `postgres` and wears its mark — but OpenSearch, which
 * the `elasticsearch` driver also reaches, has a different mark of its own:
 * keyed by driver id, an OpenSearch connection wears Elasticsearch's.
 */
const logos: Partial<Record<string, Logo>> = {
  postgres: Postgresql,
  mysql: MysqlMark,
  sqlite: SqliteMark,
  clickhouse: Clickhouse,
  duckdb: DuckdbMark,
  elasticsearch: ElasticsearchMark,
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
