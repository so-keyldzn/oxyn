import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import { DatabaseIcon } from "@hugeicons/core-free-icons"

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

/**
 * The official marks, from the svgl registry (`shadcn add @svgl/<name>`),
 * keyed by driver id. A driver per protocol (ADR-0003): Redshift reaches
 * Oxyn through `postgres` and wears its mark.
 */
const logos: Partial<Record<string, Logo>> = {
  postgres: Postgresql,
  sqlite: SqliteMark,
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
