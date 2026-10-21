import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import { Alert02Icon, ArrowDown01Icon } from "@hugeicons/core-free-icons"

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible"
import type { CatalogNode } from "@/lib/ipc/types"

/** The module sqlite-vec registers, which a connection can opt into. */
const SQLITE_VEC_MODULE = "vec0"

/**
 * Whether two module names are the same module. SQLite folds ASCII case only
 * (`USING VEC0` is `vec0`); a Unicode fold would match names the engine keeps
 * apart. Pure, so it is tested without a DOM.
 */
export function sameModule(a: string, b: string) {
  const fold = (name: string) =>
    name.replace(/[A-Z]/g, (letter) => letter.toLowerCase())
  return fold(a) === fold(b)
}

/**
 * What the failed read was about, as the notice must say it.
 *
 * `virtualTable`: the selected object **is** the virtual table backed by the
 * missing module — its shadow tables are where its data is. `dependent`:
 * anything else — a view whose definition reads such a table, say — which the
 * extension does not provide, and whose storage the notice does not claim.
 */
export type MissingModuleSubject =
  | { type: "virtualTable"; shadows: Array<string> }
  | { type: "dependent"; noun: string }

/**
 * The subject for `node`: the virtual table only when the catalog says the
 * node is one backed by `module`. Pure, so it is tested without a DOM.
 */
export function missingModuleSubject(
  node: Pick<CatalogNode, "kind" | "virtualTable">,
  module: string
): MissingModuleSubject {
  const table = node.virtualTable
  if (table && sameModule(table.module, module))
    return { type: "virtualTable", shadows: table.shadows }
  return { type: "dependent", noun: node.kind === "view" ? "view" : "object" }
}

/**
 * Why a read failed: the extension providing a module is not loaded. Shown in
 * place of the raw failure, which stays one click away — the engine's own
 * words are never replaced, only explained.
 *
 * The module and table names come from the database: rendered as text.
 */
export function MissingModuleNotice({
  module,
  subject,
  message,
  onOpenShadow,
}: {
  module: string
  subject: MissingModuleSubject
  /** The driver's message, as it came. */
  message: string
  /** Selects a shadow table, as a click in the explorer would. */
  onOpenShadow?: (name: string) => void
}) {
  const named = <code className="font-mono text-foreground">{module}</code>
  return (
    <div className="flex h-full min-h-0 flex-col overflow-auto p-4">
      <Alert className="gap-1.5 px-3 py-3">
        <HugeiconsIcon
          icon={Alert02Icon}
          strokeWidth={2}
          className="text-warning"
        />
        <AlertTitle>
          {subject.type === "virtualTable"
            ? "This table needs an SQLite extension"
            : `This ${subject.noun} reads a table that needs an SQLite extension`}
        </AlertTitle>
        <AlertDescription className="flex min-w-0 flex-col gap-2">
          {subject.type === "virtualTable" ? (
            <p>
              This table is provided by the SQLite extension {named}, which Oxyn
              does not load.{" "}
              <ShadowTables
                shadows={subject.shadows}
                onOpenShadow={onOpenShadow}
              />
            </p>
          ) : (
            <p>
              This {subject.noun} reads a virtual table whose module {named} is
              not loaded: Oxyn does not load that SQLite extension, so the read
              fails.
            </p>
          )}
          {sameModule(module, SQLITE_VEC_MODULE) ? (
            // The one extension Oxyn ships, off unless the connection opts in:
            // loading code a database file asks for is a trust decision.
            <p>
              You can enable sqlite-vec for this connection in its settings, if
              you trust this file.
            </p>
          ) : null}
          <p className="text-xs">
            Running it again as is will fail the same way.
          </p>
          <Collapsible className="flex min-w-0 flex-col">
            <CollapsibleTrigger className="group/raw inline-flex w-fit items-center gap-1 rounded-md py-0.5 pr-1 text-xs text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring">
              Driver message
              <HugeiconsIcon
                icon={ArrowDown01Icon}
                strokeWidth={2}
                className="size-3.5 shrink-0 transition-transform group-data-[panel-open]/raw:rotate-180 motion-reduce:transition-none"
                aria-hidden
              />
            </CollapsibleTrigger>
            <CollapsibleContent className="overflow-hidden">
              {/* The server's words, code included — never a paraphrase. */}
              <pre
                data-selectable
                tabIndex={0}
                aria-label="Server message"
                className="mt-1 max-h-64 overflow-auto rounded-md border bg-muted/40 p-2.5 font-mono text-xs wrap-anywhere whitespace-pre-wrap text-foreground outline-none focus-visible:ring-2 focus-visible:ring-ring"
              >
                {message}
              </pre>
            </CollapsibleContent>
          </Collapsible>
        </AlertDescription>
      </Alert>
    </div>
  )
}

/** Where a virtual table's data is: its shadow tables, as links. */
function ShadowTables({
  shadows,
  onOpenShadow,
}: {
  shadows: Array<string>
  onOpenShadow?: (name: string) => void
}) {
  if (shadows.length === 0)
    return <>No table storing its data is listed beside it.</>
  return (
    <>
      Its data is stored in{" "}
      {shadows.map((name, index) => (
        <React.Fragment key={name}>
          {index > 0 ? (index === shadows.length - 1 ? " and " : ", ") : null}
          {onOpenShadow ? (
            <Button
              variant="link"
              className="h-auto p-0 font-mono text-[length:inherit]"
              onClick={() => onOpenShadow(name)}
            >
              {name}
            </Button>
          ) : (
            <code className="font-mono text-foreground">{name}</code>
          )}
        </React.Fragment>
      ))}
      .
    </>
  )
}
