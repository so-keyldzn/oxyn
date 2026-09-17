import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import {
  ArrowDown01Icon,
  BookOpen01Icon,
  LinkSquare02Icon,
} from "@hugeicons/core-free-icons"

import { addressLabel } from "@/components/oxyn/catalog-tree"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible"
import type { CatalogAddress, PrivacyTier } from "@/lib/ipc/types"

export interface AssistantSource {
  key: string
  /** The object Oxyn read, as the catalog addresses it. */
  address: CatalogAddress
  /**
   * Which facet was read — `Structure`, `DDL`, `Indexes`, `Data`…
   *
   * A free word from the backend, shown as received: a facet added on the
   * Rust side must not need this list to be edited too.
   */
  facet: string
  /**
   * How many rows were read, `null` when none were or it was not counted.
   *
   * This is the **shape** of what was read. There is deliberately no field
   * here for a value: a component that cannot receive one cannot leak one
   * (I-03).
   */
  rows: number | null
  /** The object is no longer in the catalog: it was read, then dropped. */
  missing?: boolean
}

/**
 * What a row count means under the connection's tier.
 *
 * The tier arrives by prop, from the connection — it is never deduced here,
 * and never read from an application setting (I-04).
 */
function rowsLine(rows: number, tier: PrivacyTier) {
  const counted = `${rows.toLocaleString()} ${rows === 1 ? "row" : "rows"} read`
  return tier === "sampled"
    ? `${counted} · a sample you approved was sent`
    : `${counted} · their values stayed on this machine`
}

function SourceRow({
  source,
  tier,
  onOpenObject,
}: {
  source: AssistantSource
  tier: PrivacyTier
  onOpenObject?: (address: CatalogAddress) => void
}) {
  const label = addressLabel(source.address)
  return (
    <li
      data-slot="assistant-source"
      data-missing={source.missing ? "" : undefined}
      className="flex min-w-0 items-start gap-2"
    >
      <span className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="flex min-w-0 flex-wrap items-center gap-1.5">
          {/* `bdi` isolates the name: a right-to-left object name otherwise
              drags the dots of the address to the wrong side. */}
          <bdi className="min-w-0 font-mono wrap-break-word">{label}</bdi>
          <Badge variant="outline" className="shrink-0">
            {source.facet}
          </Badge>
        </span>
        <span className="text-xs text-muted-foreground">
          {source.rows === null
            ? "Structure only, no row read"
            : rowsLine(source.rows, tier)}
        </span>
        {source.missing ? (
          // No disabled button: an action with nothing behind it is absent,
          // and the reason is said (docs/UX-SPEC.md).
          <span className="text-xs text-muted-foreground">
            This object is no longer in the catalog. It cannot be opened; what
            Oxyn read of it may be out of date.
          </span>
        ) : null}
      </span>
      {onOpenObject && !source.missing ? (
        <Button
          size="xs"
          variant="ghost"
          className="shrink-0"
          // One row per source: ten « Open » in a row tell a screen reader
          // nothing about which object each one opens.
          aria-label={`Open ${label}`}
          onClick={() => onOpenObject(source.address)}
        >
          <HugeiconsIcon
            icon={LinkSquare02Icon}
            strokeWidth={2}
            data-icon="inline-start"
          />
          Open
        </Button>
      ) : null}
    </li>
  )
}

/**
 * What **Oxyn** read for this answer — so the answer can be checked.
 *
 * The distinction is the whole point, and the wording in this file keeps it.
 * This list is derived from what Oxyn's own tools actually executed through
 * the bus (ADR-0030): which object, which facet, how many rows. It is **not**
 * what a model claims to have used — a claimed citation is precisely the thing
 * a model can invent, which would make this list reassuring and wrong.
 *
 * So there is no protocol field behind it, and there never will be: a plan and
 * a tool call from an external agent describe work Oxyn did not authorise
 * (ADR-0026), while this describes work Oxyn ran itself and can vouch for.
 *
 * Each source shows the **shape** of what was read — the object, the facet,
 * the number of rows — never a value; whether values reached the provider is
 * decided by the connection's tier, which arrives by prop (ADR-0006, I-03).
 *
 * Opening an object navigates; it runs nothing (I-07).
 */
export function AssistantSources({
  sources,
  tier,
  defaultOpen = false,
  onOpenObject,
}: {
  /** What Oxyn's tools read, not what a model says it used. */
  sources: ReadonlyArray<AssistantSource>
  /** The connection's privacy tier. Never deduced, never an app setting. */
  tier: PrivacyTier
  defaultOpen?: boolean
  /** Opens the object's view. Absent: the list stays readable. */
  onOpenObject?: (address: CatalogAddress) => void
}) {
  const [open, setOpen] = React.useState(defaultOpen)

  // A run whose tools read nothing says so where it matters — in the answer
  // and its tier badge — not in an empty « Sources » frame that reads as a
  // loading failure.
  if (sources.length === 0) return null

  const missing = sources.filter((source) => source.missing).length

  return (
    <Collapsible
      data-slot="assistant-sources"
      open={open}
      onOpenChange={setOpen}
      className="flex min-w-0 flex-col text-sm"
    >
      <CollapsibleTrigger className="group/sources inline-flex w-fit items-center gap-1.5 rounded-md py-0.5 pr-1 text-xs text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring">
        <HugeiconsIcon
          icon={BookOpen01Icon}
          strokeWidth={2}
          className="size-3.5 shrink-0"
          aria-hidden
        />
        {/* The subject is Oxyn, not the model: the label must not promise a
            citation the answer did not make. */}
        <span className="tabular-nums">
          Oxyn read {sources.length.toLocaleString()}{" "}
          {sources.length === 1 ? "object" : "objects"}
        </span>
        {missing > 0 ? (
          <span className="tabular-nums">· {missing} no longer exist</span>
        ) : null}
        <HugeiconsIcon
          icon={ArrowDown01Icon}
          strokeWidth={2}
          className="size-3.5 shrink-0 transition-transform group-data-[panel-open]/sources:rotate-180 motion-reduce:transition-none"
          aria-hidden
        />
      </CollapsibleTrigger>
      <CollapsibleContent className="overflow-hidden">
        {/* Vertical only: a long qualified name wraps rather than pushing a
            horizontal scrollbar across a 320 px panel. */}
        <ul
          aria-label="Objects Oxyn read for this answer"
          tabIndex={0}
          className="mt-1.5 flex max-h-64 min-w-0 flex-col gap-2 overflow-x-hidden overflow-y-auto border-l-2 pl-3 text-xs leading-5 outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          {sources.map((source) => (
            <SourceRow
              key={source.key}
              source={source}
              tier={tier}
              onOpenObject={onOpenObject}
            />
          ))}
        </ul>
      </CollapsibleContent>
    </Collapsible>
  )
}
